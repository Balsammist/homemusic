"""Listening data for mixes and «Моя волна»: player events and «Не нравится» (see app/plays.py)."""

from __future__ import annotations

from datetime import date
from zoneinfo import ZoneInfoNotFoundError

from fastapi import APIRouter, Depends, HTTPException, Query
from sqlalchemy import case, func, or_, select
from sqlalchemy.ext.asyncio import AsyncSession

from app import plays
from app.api.schemas import DislikeIn, DislikeOut, PlayEventRejected, PlayEventsIn, PlayEventsOut
from app.auth import current_user
from app.db import get_session
from app.models import CatalogCache, Dislike, PlayEvent, Track, User

router = APIRouter(tags=["plays"])


@router.get("/api/plays/recent")
async def recent_plays(limit: int = Query(50, ge=1, le=200), user: User = Depends(current_user),
                       s: AsyncSession = Depends(get_session, scope="function")):
    """Recent starts for this account, with bounded local metadata enrichment only."""
    ranked = select(PlayEvent.id, func.row_number().over(
        partition_by=(PlayEvent.device_id,
                      case((PlayEvent.playback_id.is_not(None), PlayEvent.playback_id),
                           else_=PlayEvent.event_id),
                      func.coalesce(PlayEvent.ref, PlayEvent.nd_song_id)),
        order_by=(PlayEvent.ts.desc(), PlayEvent.id.desc())).label("rank")).where(
            PlayEvent.user_id == user.id, PlayEvent.kind == "start", PlayEvent.target_type == "track",
            PlayEvent.superseded_scrobble.is_(False)).subquery()
    events = list((await s.execute(select(PlayEvent).join(ranked, ranked.c.id == PlayEvent.id)
        .where(ranked.c.rank == 1).order_by(PlayEvent.ts.desc(), PlayEvent.id.desc()).limit(limit))).scalars())
    refs = {e.ref for e in events if e.ref}
    nd_ids = {e.nd_song_id for e in events if e.nd_song_id} | {r[3:] for r in refs if r.startswith("nd:")}
    dz_ids = {r[3:] for r in refs if r.startswith("dz:")}
    owned = list((await s.execute(select(Track).where(or_(
        Track.nd_song_id.in_(nd_ids), Track.deezer_track_id.in_(dz_ids))).order_by(Track.id))).scalars()) \
        if nd_ids or dz_ids else []
    by_nd = {t.nd_song_id: t for t in owned if t.nd_song_id}
    by_dz = {f"dz:{t.deezer_track_id}": t for t in owned if t.deezer_track_id}
    keys = {f"mix:track:{ref}" for ref in refs} | {f"dz:track:{r[3:]}" for r in refs if r.startswith("dz:")}
    cached = {}
    if keys:
        for row in (await s.execute(select(CatalogCache).where(CatalogCache.key.in_(keys)))).scalars():
            raw = row.payload_json or {}
            raw = raw.get("v", raw)
            if isinstance(raw, dict) and raw.get("ref") in refs:
                cached[raw["ref"]] = raw
    tracks = []
    for event in events:
        metadata = event.track_metadata if isinstance(event.track_metadata, dict) else {}
        raw = cached.get(event.ref, {})
        nd_id = event.nd_song_id or (event.ref[3:] if event.ref and event.ref.startswith("nd:") else None)
        track = by_nd.get(nd_id) or by_dz.get(event.ref)
        nd_id = track.nd_song_id if track and track.nd_song_id else nd_id
        reference = event.ref or (f"nd:{nd_id}" if nd_id else None)
        if not reference:
            continue
        artists = metadata.get("artists") or []
        artist_names = ", ".join(a.get("name", "") if isinstance(a, dict) else str(a) for a in artists)
        tracks.append({
            "ref": reference,
            "title": metadata.get("title") or (track.title if track else None) or raw.get("title") or "",
            "artist": metadata.get("artist_credit") or metadata.get("artist") or artist_names
                      or (track.artist if track else None) or raw.get("artist") or "",
            "album_title": metadata.get("album") or metadata.get("album_title")
                           or (track.album if track else None) or raw.get("album_title"),
            "duration_s": metadata.get("duration_s") or event.duration_s
                          or (track.duration_s if track else None) or raw.get("duration_s"),
            "cover_url": raw.get("cover_url"), "cover_small_url": raw.get("cover_small_url"),
            "nd_song_id": nd_id, "owned": bool(track and track.nd_song_id),
            "album_ref": raw.get("album_ref"), "artist_ref": raw.get("artist_ref"),
            "played_at": event.ts.isoformat(), "playback_id": event.playback_id,
        })
    return {"tracks": tracks}


@router.post("/api/plays/events", response_model=PlayEventsOut)
async def events(body: PlayEventsIn, user: User = Depends(current_user),
                 s: AsyncSession = Depends(get_session, scope="function")):
    if body.expected_user_id is not None and body.expected_user_id != user.id:
        raise HTTPException(409, "Listening events belong to another account")
    r = await plays.record_events(s, user.id, body.events)
    # a catalog track the app played from its precache never asked the stream as a listen: its `start` is one
    # (a track of an economy import is downloaded once listened to)
    refs = {e.get("ref") for e in body.events if isinstance(e, dict) and e.get("kind") == "start"}
    if refs := sorted(r for r in refs if isinstance(r, str) and r and not r.startswith("nd:")):
        from app.imports.core import listened

        await s.commit()
        for ref in refs:
            await listened(user.id, ref)
    return PlayEventsOut(accepted=r.accepted, duplicates=r.duplicates,
                         rejected=[PlayEventRejected(**x) for x in r.rejected])


@router.get("/api/plays/summary")
async def listening_summary(start: date, end: date, timezone: str = "UTC", user: User = Depends(current_user),
                            s: AsyncSession = Depends(get_session, scope="function")):
    from app.listening_summary import summary

    if end < start or (end - start).days > 3660 or end == date.max:
        raise HTTPException(422, "Invalid listening period")
    try:
        return await summary(s, user.id, start, end, timezone)
    except (ZoneInfoNotFoundError, ValueError):
        raise HTTPException(422, "Unknown timezone") from None


def _out(d: Dislike) -> DislikeOut:
    return DislikeOut(id=d.id, ref=d.ref, nd_song_id=d.nd_song_id, artist_ref=d.artist_ref, artist=d.artist,
                      title=d.title, source=d.source, created_at=d.created_at)


@router.get("/api/dislikes", response_model=list[DislikeOut])
async def list_dislikes(user: User = Depends(current_user), s: AsyncSession = Depends(get_session, scope="function")):
    rows = (await s.execute(select(Dislike).where(Dislike.user_id == user.id)
                            .order_by(Dislike.created_at.desc(), Dislike.id.desc()))).scalars()
    return [_out(d) for d in rows]


@router.post("/api/dislikes", response_model=DislikeOut)
async def add(body: DislikeIn, user: User = Depends(current_user),
              s: AsyncSession = Depends(get_session, scope="function")):
    d = await plays.add_dislike(s, user.id, ref=body.ref, nd_song_id=body.nd_song_id, artist_ref=body.artist_ref,
                                artist=body.artist, title=body.title, source=plays.dislike_source(body.client))
    assert d is not None
    await plays.dislike_event(s, user.id, "dislike", ref=d.artist_ref or d.ref, nd_song_id=d.nd_song_id,
                              client=body.client, context=body.context,
                              target_type="artist" if d.artist_ref else "track")
    return _out(d)


@router.delete("/api/dislikes")
async def remove(ref: str | None = None, nd_song_id: str | None = None, artist_ref: str | None = None,
                 client: str = "pwa", user: User = Depends(current_user),
                 s: AsyncSession = Depends(get_session, scope="function")):
    """«Не нравится» off: the track (`ref` and/or `nd_song_id`) or the artist (`artist_ref`)."""
    if not (ref or nd_song_id or artist_ref):
        raise HTTPException(400, "Укажите трек или исполнителя")
    n = await plays.remove_dislikes(s, user.id, ref=ref, nd_song_id=nd_song_id, artist_ref=artist_ref)
    if n:
        await plays.dislike_event(s, user.id, "undislike", ref=artist_ref or ref, nd_song_id=nd_song_id,
                                  client=client[:32], target_type="artist" if artist_ref else "track")
    return {"removed": n}


@router.delete("/api/dislikes/{dislike_id}")
async def remove_one(dislike_id: int, user: User = Depends(current_user),
                     s: AsyncSession = Depends(get_session, scope="function")):
    d = await s.get(Dislike, dislike_id)
    if d is None or d.user_id != user.id:
        raise HTTPException(404, "Не найдено")
    await plays.dislike_event(s, user.id, "undislike", ref=d.artist_ref or d.ref, nd_song_id=d.nd_song_id,
                              client="pwa", target_type="artist" if d.artist_ref else "track")
    await s.delete(d)
    return {"removed": 1}
