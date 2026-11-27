"""Import decisions against an isolated in-memory DB; never access live account data."""
import unittest
from unittest.mock import AsyncMock, patch

from sqlalchemy.ext.asyncio import async_sessionmaker, create_async_engine

from app.api.schemas import FavoriteTarget
from app.favorites import record
from app.imports import apply, core
from app.models import Base, Import, ImportCollection, ImportItem, Track, User


class ImportFavoriteIntentTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.engine = create_async_engine("sqlite+aiosqlite:///:memory:")
        async with self.engine.begin() as connection:
            await connection.run_sync(Base.metadata.create_all)
        self.session = async_sessionmaker(self.engine, expire_on_commit=False)()
        self.user = User(nd_username="test", nd_password_enc="unused")
        self.session.add(self.user)
        await self.session.flush()
        self.imp = Import(user_id=self.user.id, source="spotify", method="export", status="running",
                          mode="economy", options_json={})
        self.session.add(self.imp)
        await self.session.flush()

    async def asyncTearDown(self):
        await self.session.close()
        await self.engine.dispose()

    async def add_item(self, ref, *, status="waiting", selected=True, kind="likes", track_id=None):
        coll = ImportCollection(import_id=self.imp.id, kind=kind, external_id=ref, title="Test",
                                selected=selected, position=0, nd_song_ids=[])
        self.session.add(coll)
        await self.session.flush()
        item = ImportItem(import_id=self.imp.id, collection_id=coll.id, external_id=ref, ref=ref,
                          status=status, match="exact", position=0, track_id=track_id,
                          raw_json={"artist": "Test", "title": "Track"})
        self.session.add(item)
        await self.session.flush()
        return item

    async def test_pending_hearts_survive_failed_download_and_respect_unlikes(self):
        await self.add_item("dz:1")
        failed = await self.add_item("dz:2", status="failed")
        await self.add_item("dz:3", selected=False)
        await self.add_item("dz:4", kind="playlist")
        self.assertEqual(set(await apply.pending_like_refs(self.session, self.user.id)), {"dz:1", "dz:2"})
        await record(self.session, self.user.id, FavoriteTarget(ref="dz:1"), False)
        self.assertEqual(await apply.pending_like_refs(self.session, self.user.id), ["dz:2"])
        await apply.unlike_refs(self.session, self.user.id, ["dz:2"])
        self.assertEqual(failed.status, "skipped")
        self.assertEqual(await apply.pending_like_refs(self.session, self.user.id), [])

    async def test_reimport_does_not_star_an_explicitly_unliked_owned_song(self):
        song = Track(artist="Test", title="Track", nd_song_id="song1", path="test.flac",
                     uid="test-uid", fingerprint="test|track")
        self.session.add(song)
        await self.session.flush()
        item = await self.add_item("dz:1", status="done", track_id=song.id)
        await record(self.session, self.user.id, FavoriteTarget(ref="dz:1", nd_song_id="song1"), False)
        nd = AsyncMock()
        with patch.object(apply, "_nd", return_value=nd):
            await apply.sync_likes(self.session, self.imp)
        nd.star.assert_not_awaited()
        self.assertEqual(item.status, "skipped")
        self.assertTrue(item.raw_json["unliked"])

    async def test_owned_import_is_starred_without_creating_an_own_decision(self):
        song = Track(artist="Test", title="Track", nd_song_id="song1", path="test.flac",
                     uid="test-uid", fingerprint="test|track")
        self.session.add(song)
        await self.session.flush()
        await self.add_item("dz:1", status="done", track_id=song.id)
        nd = AsyncMock()
        with patch.object(apply, "_nd", return_value=nd):
            await apply.sync_likes(self.session, self.imp)
        nd.star.assert_awaited_once_with(song_ids=["song1"])
        self.assertEqual(self.imp.options_json["liked"], ["song1"])
        from app.favorites import snapshot
        self.assertEqual(await snapshot(self.session, self.user.id), {})

    async def test_download_all_keeps_unselected_collections_out(self):
        wanted = await self.add_item("dz:1")
        await self.add_item("dz:2", selected=False)
        queue = AsyncMock(return_value=True)
        with patch.object(apply, "queue_track", queue):
            count = await core.download_all(self.session, self.imp)
        self.assertEqual(count, 1)
        queue.assert_awaited_once_with(self.session, self.imp, wanted, "likes")


if __name__ == "__main__":
    unittest.main()
