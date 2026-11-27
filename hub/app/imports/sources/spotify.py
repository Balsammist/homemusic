"""Read Spotify's account JSON export or Exportify CSV/ZIP, without account tokens or Premium.

Only music files are read from an archive, in memory. No archive member is extracted.
Playlist order and repeated tracks are retained; likes are de-duplicated by Spotify URI.
"""
from __future__ import annotations

import csv
import hashlib
import io
import json
import math
import re
import zipfile
from collections import Counter
from pathlib import PurePosixPath
from typing import Any

from .base import SnapCollection, SnapItem, Snapshot, SourceError

MAX_BYTES = 50 * 1024 * 1024
MAX_FILES = 512
MAX_ITEMS = 100_000
LIKES = {"liked", "liked_songs", "liked songs", "liked tracks", "liked_tracks", "yourlibrary", "your_library",
         "любимые треки", "понравившиеся треки", "понравившиеся_треки", "мне нравится"}


def _str(value: Any) -> str:
    return value.strip() if isinstance(value, str) else ""


def _id(uri: str, raw: dict[str, Any]) -> str:
    if re.fullmatch(r"spotify:track:[A-Za-z0-9]+", uri):
        return uri.split(":")[-1]
    return "names-" + hashlib.sha256((raw["artist"] + "\0" + raw["title"] + "\0" +
                                      (raw.get("album") or "")).encode()).hexdigest()[:24]


def _track(row: dict[str, Any]) -> tuple[str, dict[str, Any]] | None:
    t = row.get("track") or row.get("localTrack") or row
    if not isinstance(t, dict):
        # YourLibrary.json uses a string in the "track" field.
        t = row
    if t.get("type") in ("episode", "show") or row.get("episode"):
        return None
    title = _str(t.get("trackName") or t.get("Track Name") or t.get("name") or t.get("track"))
    artist = _str(t.get("artistName") or t.get("Artist Name(s)") or t.get("artist"))
    artists = t.get("artists")
    names = [x["name"].strip() for x in artists if isinstance(x, dict) and _str(x.get("name"))] \
        if isinstance(artists, list) else []
    if not artist:
        artist = ", ".join(names)
    if not names and ";" in artist:
        names = [name.strip() for name in artist.split(";") if name.strip()]
        artist = ", ".join(names)
    if not artist or not title:
        return None
    album_obj = t.get("album")
    album = _str(t.get("albumName") or t.get("Album Name") or
                 (album_obj.get("name") if isinstance(album_obj, dict) else album_obj))
    uri = _str(t.get("uri") or t.get("trackUri") or t.get("Track URI"))
    if not uri and _str(t.get("id")):
        uri = "spotify:track:" + t["id"]
    raw = {"artist": artist, "artists": names or [artist], "title": title}
    if album:
        raw["album"] = album
    if uri:
        raw["spotify_uri"] = uri
    duration = t.get("duration_ms") or t.get("Track Duration (ms)")
    if duration:
        try:
            seconds = float(duration) / 1000
            if math.isfinite(seconds) and 0 < seconds <= 86400:
                raw["duration_s"] = round(seconds)
        except (TypeError, ValueError, OverflowError):
            pass
    ids = t.get("external_ids")
    isrc = _str(t.get("ISRC") or (ids.get("isrc") if isinstance(ids, dict) else None))
    if isrc:
        raw["isrc"] = isrc
    added = _str(row.get("addedDate") or row.get("added_at") or row.get("Added At"))
    if added:
        raw["liked_at"] = added
    return _id(uri, raw), raw


def _collection(kind: str, key: str, title: str, rows: list) -> SnapCollection:
    if len(rows) > MAX_ITEMS:
        raise SourceError("Слишком много треков в одном файле (максимум 100 000)")
    counts: Counter[str] = Counter()
    items = []
    for row in rows:
        if not isinstance(row, dict) or not (track := _track(row)):
            continue
        ext, raw = track
        counts[ext] += 1
        if counts[ext] > 1 and kind == "likes":
            continue
        # The existing importer keys items by collection + external_id. Keep repeated playlist positions distinct.
        item_id = ext if counts[ext] == 1 else f"{ext}~{counts[ext]}"
        items.append(SnapItem(item_id, raw))
    return SnapCollection(kind=kind, external_id=key, title=title[:512], items=items)


def _csv(name: str, data: bytes) -> list[SnapCollection]:
    try:
        reader = csv.DictReader(io.StringIO(data.decode("utf-8-sig"), newline=""))
        fields = set(reader.fieldnames or [])
        if not {"Track Name", "Artist Name(s)"} <= fields:
            return []  # e.g. Exportify saved_albums.csv; not a track playlist
        rows = []
        for row in reader:
            rows.append(row)
            if len(rows) > MAX_ITEMS:
                raise SourceError("Слишком много треков в CSV (максимум 100 000)")
        stem = PurePosixPath(name).stem
        liked = stem.casefold() in LIKES
        key = "likes" if liked else "playlist-" + hashlib.sha256(name.encode()).hexdigest()[:24]
        return [_collection("likes" if liked else "playlist", key, "Любимые треки" if liked else stem, rows)]
    except (UnicodeError, csv.Error):
        raise SourceError("Не удалось прочитать CSV. Нужен UTF-8 экспорт Exportify.") from None


def _json(name: str, data: bytes) -> list[SnapCollection]:
    try:
        obj = json.loads(data)
    except (ValueError, UnicodeError, RecursionError):
        raise SourceError("Файл не содержит корректный JSON экспорта Spotify") from None
    if not isinstance(obj, dict):
        return []  # streaming history and unrelated account data are deliberately ignored
    result = []
    stem = PurePosixPath(name).stem.casefold()
    if stem in LIKES and isinstance(obj.get("tracks"), list):
        result.append(_collection("likes", "likes", "Любимые треки", obj["tracks"]))
    playlists = obj.get("playlists")
    if not isinstance(playlists, list):
        playlists = [obj] if "name" in obj and "tracks" in obj else []
    for pos, playlist in enumerate(playlists):
        if not isinstance(playlist, dict):
            continue
        title = _str(playlist.get("name")) or "Плейлист Spotify"
        rows = playlist.get("items") or playlist.get("tracks") or []
        if isinstance(rows, dict):
            rows = rows.get("items", [])
        if not isinstance(rows, list):
            continue
        key = _str(playlist.get("uri") or playlist.get("id")) or "playlist-" + hashlib.sha256(
            f"{name}\0{pos}\0{title}".encode()).hexdigest()[:24]
        result.append(_collection("playlist", key, title, rows))
    return result


def parse_export(filename: str, data: bytes) -> Snapshot:
    if len(data) > MAX_BYTES:
        raise SourceError("Экспорт больше 50 МБ. Выберите отдельные файлы библиотеки.")
    files = []
    if data.startswith(b"PK"):
        try:
            with zipfile.ZipFile(io.BytesIO(data)) as archive:
                infos = archive.infolist()
                if len(infos) > MAX_FILES or sum(x.file_size for x in infos) > MAX_BYTES:
                    raise SourceError("Распакованный экспорт слишком большой (максимум 50 МБ и 512 файлов)")
                for info in infos:
                    p = PurePosixPath(info.filename)
                    if p.suffix.lower() not in (".csv", ".json") or info.is_dir():
                        continue
                    if p.is_absolute() or ".." in p.parts or "\\" in info.filename or info.flag_bits & 1:
                        raise SourceError("Архив содержит неподдерживаемые пути или зашифрованные файлы")
                    files.append((info.filename, archive.read(info)))
        except (zipfile.BadZipFile, OSError, RuntimeError, NotImplementedError):
            raise SourceError("Не удалось прочитать ZIP экспорта Spotify") from None
    else:
        files = [(PurePosixPath(filename).name, data)]
    collections = []
    total = 0
    for name, content in files:
        suffix = PurePosixPath(name).suffix.lower()
        parsed = _csv(name, content) if suffix == ".csv" else _json(name, content) if suffix == ".json" else []
        for coll in parsed:
            total += len(coll.items)
            if total > MAX_ITEMS:
                raise SourceError("В экспорте больше 100 000 треков. Перенесите плейлисты по отдельности.")
            if coll.items:
                collections.append(coll)
    if not collections:
        raise SourceError("Треки не найдены. Выберите ZIP/CSV Exportify или YourLibrary.json / Playlist*.json Spotify.")
    if len({c.external_id for c in collections}) != len(collections):
        raise SourceError("В архиве несколько одинаковых коллекций. Выберите один экспорт библиотеки.")
    return Snapshot(account="Spotify", collections=collections)
