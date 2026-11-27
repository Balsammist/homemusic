import csv
import io
import json
import unittest
import zipfile
from unittest.mock import patch

from app.imports.sources import spotify
from app.imports.sources.base import SourceError


def csv_bytes(rows):
    out = io.StringIO(newline="")
    writer = csv.DictWriter(out, fieldnames=["Track URI", "Track Name", "Artist Name(s)", "Album Name",
                                           "Track Duration (ms)", "Added At", "ISRC"])
    writer.writeheader()
    for ident, title in rows:
        writer.writerow({"Track URI": f"spotify:track:{ident}", "Track Name": title,
                         "Artist Name(s)": "Исполнитель, другой", "Album Name": "Альбом",
                         "Track Duration (ms)": "211500", "Added At": "2025-01-01T12:00:00Z", "ISRC": "TEST12345678"})
    return out.getvalue().encode("utf-8-sig")


def archive(files):
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", compression=zipfile.ZIP_DEFLATED) as z:
        for name, data in files:
            z.writestr(name, data)
    return out.getvalue()


class SpotifyExportTests(unittest.TestCase):
    def test_exportify_liked_filename_is_favorites_not_a_playlist(self):
        for name in ("liked.csv", "LIKED.csv", "Spotify/liked.csv"):
            with self.subTest(name=name):
                coll = spotify.parse_export(name, csv_bytes([("A", "One")])).collections[0]
                self.assertEqual((coll.kind, coll.external_id), ("likes", "likes"))

    def test_exportify_likes_unicode_bom_and_deduplication(self):
        snap = spotify.parse_export("liked_songs.csv", csv_bytes([("A1", "Трек"), ("A1", "Трек")]))
        coll = snap.collections[0]
        self.assertEqual((coll.kind, coll.title, len(coll.items)), ("likes", "Любимые треки", 1))
        self.assertEqual(coll.items[0].raw["title"], "Трек")
        self.assertEqual(coll.items[0].raw["duration_s"], 212)
        self.assertEqual(coll.items[0].raw["isrc"], "TEST12345678")

    def test_playlist_order_and_repeated_tracks_are_retained(self):
        coll = spotify.parse_export("Дорога.csv", csv_bytes([("A", "Первый"), ("B", "Второй"), ("A", "Первый")])).collections[0]
        self.assertEqual(coll.kind, "playlist")
        self.assertEqual([x.raw["title"] for x in coll.items], ["Первый", "Второй", "Первый"])
        self.assertEqual(len({x.external_id for x in coll.items}), 3)
        self.assertEqual(coll.items[0].raw["spotify_uri"], coll.items[2].raw["spotify_uri"])

    def test_spotify_account_library_json(self):
        data = {"tracks": [{"artist": "A", "album": "B", "track": "C", "uri": "spotify:track:abc123"}]}
        coll = spotify.parse_export("YourLibrary.json", json.dumps(data).encode()).collections[0]
        self.assertEqual(coll.kind, "likes")
        self.assertEqual(coll.items[0].external_id, "abc123")
        self.assertEqual(coll.items[0].raw["artist"], "A")

    def test_spotify_playlist_json_skips_podcasts_and_preserves_order(self):
        data = {"playlists": [{"name": "Mix", "items": [
            {"track": {"trackName": "One", "artistName": "A", "albumName": "B", "trackUri": "spotify:track:one"}},
            {"episode": {"episodeName": "Podcast"}},
            {"track": {"trackName": "Two", "artistName": "A", "trackUri": "spotify:track:two"}}]}]}
        coll = spotify.parse_export("Playlist1.json", json.dumps(data).encode()).collections[0]
        self.assertEqual(coll.title, "Mix")
        self.assertEqual([x.raw["title"] for x in coll.items], ["One", "Two"])

    def test_archive_ignores_unrelated_account_identity_and_history(self):
        data = archive([("Spotify Account Data/Userdata.json", b'{"email":"not-imported@example.test"}'),
                        ("Spotify Account Data/StreamingHistory0.json", b'[{"trackName":"not-a-like"}]'),
                        ("liked_songs.csv", csv_bytes([("A", "One")])),
                        ("my-playlist.csv", csv_bytes([("B", "Two")]))])
        snap = spotify.parse_export("export.zip", data)
        self.assertEqual([x.kind for x in snap.collections], ["likes", "playlist"])
        self.assertNotIn("not-imported", repr(snap))

    def test_archive_path_traversal_is_rejected_without_extraction(self):
        with self.assertRaises(SourceError):
            spotify.parse_export("bad.zip", archive([("../../liked_songs.csv", csv_bytes([("A", "One")]))]))

    def test_zip_expansion_and_item_limits(self):
        with patch.object(spotify, "MAX_BYTES", 2048):
            with self.assertRaises(SourceError):
                spotify.parse_export("large.zip", archive([("x.json", b" " * 5000)]))
        with patch.object(spotify, "MAX_ITEMS", 2):
            with self.assertRaises(SourceError):
                spotify.parse_export("x.csv", csv_bytes([("A", "1"), ("B", "2"), ("C", "3")]))

    def test_wrong_file_and_malformed_json_are_actionable_errors(self):
        for name, data in [("data.json", b"broken"), ("Userdata.json", b'{"name":"A"}'),
                           ("wrong.csv", b"name,date\nA,B\n"), ("x.zip", b"PKbroken")]:
            with self.subTest(name=name), self.assertRaises(SourceError):
                spotify.parse_export(name, data)

    def test_ambiguous_duplicate_like_exports_rejected(self):
        data = archive([("liked_songs.csv", csv_bytes([("A", "One")])),
                        ("Liked Songs.csv", csv_bytes([("B", "Two")]))])
        with self.assertRaises(SourceError):
            spotify.parse_export("duplicate.zip", data)


if __name__ == "__main__":
    unittest.main()
