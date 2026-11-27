"""Read-only recent playback response; isolated DB, no catalog network lookups."""
import unittest
from datetime import datetime, timedelta

from sqlalchemy.ext.asyncio import async_sessionmaker, create_async_engine

from app.api.plays import recent_plays
from app.models import Base, CatalogCache, PlayEvent, Track, User


class RecentPlaybackTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.engine = create_async_engine("sqlite+aiosqlite:///:memory:")
        async with self.engine.begin() as connection:
            await connection.run_sync(Base.metadata.create_all)
        self.s = async_sessionmaker(self.engine, expire_on_commit=False)()
        self.user = User(nd_username="one", nd_password_enc="unused")
        self.other = User(nd_username="two", nd_password_enc="unused")
        self.s.add_all([self.user, self.other])
        await self.s.flush()

    async def asyncTearDown(self):
        await self.s.close()
        await self.engine.dispose()

    async def add_event(self, eid, seconds, **extra):
        fields = dict(user_id=self.user.id, event_id=eid, kind="start", target_type="track",
                      client="native", ts=datetime(2026, 1, 1) + timedelta(seconds=seconds),
                      ref="dz:1", device_id="device", playback_id="playback", track_metadata={"title": "Snapshot"})
        fields.update(extra)
        self.s.add(PlayEvent(**fields))
        await self.s.flush()

    async def test_account_isolation_and_distinct_repeated_playbacks(self):
        await self.add_event("first", 1)
        await self.add_event("retry-start", 2)
        await self.add_event("repeat-song", 3, playback_id="second-playback")
        await self.add_event("other-user", 4, user_id=self.other.id, ref="dz:2")
        await self.add_event("finish", 5, kind="finish")
        await self.add_event("like", 6, kind="like")
        result = await recent_plays(limit=200, user=self.user, s=self.s)
        self.assertEqual(len(result["tracks"]), 2)
        self.assertEqual([t["playback_id"] for t in result["tracks"]], ["second-playback", "playback"])
        self.assertTrue(all(t["ref"] == "dz:1" for t in result["tracks"]))
        limited = await recent_plays(limit=1, user=self.user, s=self.s)
        self.assertEqual(len(limited["tracks"]), 1)

    async def test_snapshot_and_wrapped_cache_enrich_owned_track_without_writes(self):
        self.s.add(Track(uid="track", path="test.flac", fingerprint="artist|title", artist="Library Artist",
                         title="Library Title", album="Library Album", duration_s=180,
                         nd_song_id="song", deezer_track_id="1"))
        self.s.add(CatalogCache(key="dz:track:1", ttl_s=1, payload_json={"v": {
            "ref": "dz:1", "cover_url": "https://example.test/cover", "artist_ref": "dz:10"}}))
        await self.s.flush()
        await self.add_event("snapshot", 1)
        result = await recent_plays(limit=10, user=self.user, s=self.s)
        track = result["tracks"][0]
        self.assertEqual(track["title"], "Snapshot")
        self.assertEqual(track["artist"], "Library Artist")
        self.assertEqual(track["album_title"], "Library Album")
        self.assertEqual(track["duration_s"], 180)
        self.assertEqual(track["cover_url"], "https://example.test/cover")
        self.assertEqual(track["artist_ref"], "dz:10")
        self.assertEqual(track["nd_song_id"], "song")
        self.assertTrue(track["owned"])
        self.assertFalse(self.s.new or self.s.dirty or self.s.deleted)


if __name__ == "__main__":
    unittest.main()
