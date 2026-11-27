"""Read-only personalized groups. No genre/language guesses from artist names."""
from collections import defaultdict


def group_tracks(profile, *, day_key=None):
    from app.mixes.taste_select import facet_names
    from app.mixes.taste import artist_key
    positive = {t.key: t for t in profile.tracks.values()
                if t.score > 0 and (t.ref or t.nd_song_id)}
    def ordered(members):
        if day_key is None:
            return sorted(members, key=lambda t: (-t.score, t.key))
        import hashlib
        import math

        def priority(t):
            digest = hashlib.sha256(f"{day_key}|{t.key}".encode()).digest()
            uniform = (int.from_bytes(digest[:8], "big") + 1) / (2**64 + 1)
            return -math.log(uniform) / max(t.score, 0.001), t.key

        return sorted(members, key=priority)

    groups = []
    used = set()
    if getattr(profile, "geometry", None):
        for facet in sorted(facet_names(profile), key=lambda f: (-f["weight"], f["id"])):
            members = ordered(positive[k] for k in facet["members"] if k in positive and k not in used)
            if len(members) >= 2:
                groups.append({"id": facet["id"], "title": " / ".join(facet["artists"]) or members[0].artist,
                               "members": members[:40]})
                used.update(t.key for t in members)
            if len(groups) == 6:
                break
    if not groups:
        artists = defaultdict(list)
        for t in positive.values():
            artists[artist_key(t.artist)].append(t)
        for artist, members in sorted(artists.items(), key=lambda kv: (-sum(t.score for t in kv[1]), kv[0])):
            if len(members) >= 2:
                selected = ordered(members)[:40]
                groups.append({"id": f"artist:{artist}", "title": selected[0].artist,
                               "members": selected})
            if len(groups) == 6:
                break
    # Round-robin across actual groups: no decorative empty directions.
    if len(groups) > 1:
        mixed, seen = [], set()
        for i in range(max(len(g["members"]) for g in groups)):
            for g in groups:
                if i < len(g["members"]) and g["members"][i].key not in seen:
                    t = g["members"][i]
                    mixed.append(t)
                    seen.add(t.key)
        groups.insert(0, {"id": "supermix", "title": "Супермикс", "members": mixed[:120]})
    return groups


async def hydrate_tracks(groups, nd):
    """Restore presentation records omitted by taste math, once per selected identity.

    Taste scores and canonical refs remain untouched. A provider outage retains the
    original member and its known metadata; missing duration never implies bad audio.
    """
    import asyncio
    from urllib.parse import quote

    from app.catalog.models import CatalogTrack, split_ref
    from app.mixes.resolve import catalog_track

    members = {t.key: t for group in groups for t in group["members"]}
    semaphore = asyncio.Semaphore(4)

    async def one(t):
        async with semaphore:
            record = t.track
            if record is None and t.ref:
                record = await catalog_track(t.ref)
            ref = t.ref or f"nd:{t.nd_song_id}"
            source, id_ = split_ref(ref)
            record = record.model_copy(deep=True) if record else CatalogTrack(
                ref=ref, source=source, id=id_, title=t.title, artist=t.artist, duration_s=t.duration_s)
            if t.nd_song_id and (not record.cover_url or not record.title or not record.artist
                                 or not record.duration_s):
                try:
                    song = await nd.get_song(t.nd_song_id)
                except Exception:
                    song = {}  # Preserve the member on a transient local-library failure.
                record.title = record.title or song.get("title") or ""
                record.artist = record.artist or song.get("artist") or ""
                record.duration_s = record.duration_s or song.get("duration")
                record.album_title = record.album_title or song.get("album")
                if not record.album_ref and song.get("albumId"):
                    record.album_ref = f"nd:{song['albumId']}"
                if not record.cover_url and song.get("coverArt"):
                    cover = quote(str(song["coverArt"]), safe="")
                    record.cover_url = f"/api/nd/cover/{cover}?size=1000"
                    record.cover_small_url = f"/api/nd/cover/{cover}?size=300"
            return t.key, record

    return dict(await asyncio.gather(*(one(t) for t in members.values())))
