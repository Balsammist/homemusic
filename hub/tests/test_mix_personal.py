"""Personal groups preserve real taste directions and deterministic interleaving."""
from types import SimpleNamespace

from app.mixes.personal import group_tracks
from app.mixes.taste import TasteTrack


def profile(*artists, geometry=None):
    tracks = {}
    for n, artist in enumerate(artists):
        key = f"dz:{n}"
        tracks[key] = TasteTrack(key=key, ref=key, artist=artist, title=f"Track {n}", score=10-n/100)
    return SimpleNamespace(tracks=tracks, geometry=geometry)


def test_artist_fallback_normalizes_case_and_supermix_interleaves():
    p = profile("Alpha", "alpha", "Beta", "Beta", "Singleton")
    groups = group_tracks(p)
    assert len(groups) == 3
    assert groups[0]["id"] == "supermix"
    assert [t.artist.lower() for t in groups[0]["members"]] == ["alpha", "beta", "alpha", "beta"]
    assert len({t.key for t in groups[0]["members"]}) == 4
    assert "singleton" not in {g["id"] for g in groups}


def test_no_decorative_groups_for_sparse_or_negative_taste():
    p = profile("Only", "Other")
    assert group_tracks(p) == []
    p.tracks["dz:1"].artist = "Only"
    p.tracks["dz:1"].score = -1
    assert group_tracks(p) == []


def test_facets_are_disjoint_bounded_and_supermix_varies_groups(monkeypatch):
    p = profile(*[f"Artist {i//2}" for i in range(16)], geometry=True)
    facets = [{"id": f"facet-{i}", "weight": 10-i, "artists": [f"Artist {i}"],
               "members": [f"dz:{2*i}", f"dz:{2*i+1}"]} for i in range(8)]
    facets[1]["members"].append("dz:0")
    monkeypatch.setattr("app.mixes.taste_select.facet_names", lambda p: facets)
    groups = group_tracks(p)
    assert len(groups) == 7 and groups[0]["id"] == "supermix"
    members = [t.key for g in groups[1:] for t in g["members"]]
    assert len(members) == len(set(members)) == 12
    assert [t.artist for t in groups[0]["members"][:6]] == [f"Artist {i}" for i in range(6)]


def test_unusable_facets_fall_back_to_actual_artist_groups(monkeypatch):
    p = profile("Alpha", "Alpha", geometry=True)
    monkeypatch.setattr("app.mixes.taste_select.facet_names", lambda p: [])
    assert [g["title"] for g in group_tracks(p)] == ["Alpha"]


async def test_hydration_restores_catalog_artwork_once_and_keeps_canonical_ref(monkeypatch):
    from app.catalog.models import CatalogTrack
    from app.mixes.personal import hydrate_tracks
    calls = []

    async def catalog(ref):
        calls.append(ref)
        return CatalogTrack(ref=ref, source="dz", id="1", artist="Alpha", title="Song",
                            duration_s=234, album_ref="dz:7", cover_url="https://example.test/art")

    monkeypatch.setattr("app.mixes.resolve.catalog_track", catalog)
    t = TasteTrack(key="nd:local", ref="dz:1", nd_song_id="local", score=1)
    groups = [{"members": [t]}, {"members": [t]}]
    records = await hydrate_tracks(groups, None)
    assert calls == ["dz:1"]
    assert records[t.key].ref == "dz:1"
    assert records[t.key].duration_s == 234
    assert records[t.key].cover_url == "https://example.test/art"
    assert t.title == ""  # Presentation hydration cannot alter taste inputs.


async def test_hydration_uses_actual_local_metadata_and_survives_unavailability():
    from app.mixes.personal import hydrate_tracks

    class Local:
        async def get_song(self, id_):
            if id_ == "missing":
                raise ConnectionError("offline")
            return {"title": "Local song", "artist": "Local artist", "duration": 192,
                    "album": "Local album", "albumId": "album", "coverArt": "art/id"}

    tracks = [TasteTrack(key=f"nd:{id_}", nd_song_id=id_, score=1) for id_ in ("song", "missing")]
    records = await hydrate_tracks([{"members": tracks}], Local())
    song = records["nd:song"]
    assert (song.title, song.artist, song.duration_s) == ("Local song", "Local artist", 192)
    assert song.album_ref == "nd:album" and song.cover_small_url == "/api/nd/cover/art%2Fid?size=300"
    assert records["nd:missing"].title == "" and records["nd:missing"].duration_s is None
    assert len(records) == 2


def test_daily_selection_is_stable_personal_and_changes_membership():
    p = profile(*(["Alpha"] * 100))
    a = group_tracks(p, day_key="1:2026-11-14")[0]["members"]
    again = group_tracks(p, day_key="1:2026-11-14")[0]["members"]
    tomorrow = group_tracks(p, day_key="1:2026-11-15")[0]["members"]
    other_user = group_tracks(p, day_key="2:2026-11-14")[0]["members"]
    assert [t.key for t in a] == [t.key for t in again]
    assert {t.key for t in a} != {t.key for t in tomorrow}
    assert [t.key for t in a] != [t.key for t in other_user]
    assert len(a) == 40 and all(t.score > 0 for t in a)
