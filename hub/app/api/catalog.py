"""Spotify-like artist and album pages with library ownership (FR-4)."""

from __future__ import annotations

from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy.ext.asyncio import AsyncSession

from app.api.schemas import AlbumOut, ArtistPageOut, TrackOut
from app.auth import current_user, user_navidrome
from app.catalog.models import ArtistPage, CatalogAlbum, CatalogTrack, split_ref
from app.db import get_session
from app.library.owned import annotate_albums, annotate_tracks
from app.models import User
from app.services import svc

router = APIRouter(prefix="/api/catalog", tags=["catalog"])


def _split(ref: str) -> tuple[str, str]:
    try:
        return split_ref(ref)
    except ValueError as e:
        raise HTTPException(400, "Неверная ссылка на каталог") from e


async def get_artist_page(ref: str) -> ArtistPage:
    src, id_ = _split(ref)
    try:
        if src == "dz" and svc.deezer:
            return await svc.deezer.get_artist_page(id_)
        if src == "hm" and svc.hitmoz_catalog:
            return await svc.hitmoz_catalog.get_artist(id_)
    except HTTPException:
        raise
    except Exception as e:
        if type(e).__name__ == "CatalogNotFound":
            raise HTTPException(404, "Исполнитель не найден") from e
        raise HTTPException(502, f"Каталог недоступен: {e}") from e
    raise HTTPException(404, "Исполнитель не найден")


async def get_album(ref: str) -> CatalogAlbum:
    src, id_ = _split(ref)
    try:
        if src == "dz" and svc.deezer:
            return await svc.deezer.get_album(id_)
        if src == "hm" and svc.hitmoz_catalog:
            return await svc.hitmoz_catalog.get_album(id_)
    except Exception as e:
        if type(e).__name__ == "CatalogNotFound":
            raise HTTPException(404, "Альбом не найден") from e
        raise HTTPException(502, f"Каталог недоступен: {e}") from e
    raise HTTPException(404, "Альбом не найден")


@router.get("/artist/{ref}", response_model=ArtistPageOut)
async def artist(ref: str, user: User = Depends(current_user), s: AsyncSession = Depends(get_session, scope="function")):
    page = await get_artist_page(ref)
    albums = await annotate_albums(s, page.albums, user.id)
    albums.sort(key=lambda a: (a.release_date is not None, a.release_date), reverse=True)
    return ArtistPageOut(
        artist=page.artist,
        top_tracks=await annotate_tracks(s, page.top_tracks, user.id),
        albums=[a for a in albums if a.record_type == "album"],
        singles=[a for a in albums if a.record_type in ("single", "ep")],
        compilations=[a for a in albums if a.record_type == "compile"],
    )


@router.get("/album/{ref}", response_model=AlbumOut)
async def album(ref: str, user: User = Depends(current_user), s: AsyncSession = Depends(get_session, scope="function")):
    a = await get_album(ref)
    return (await annotate_albums(s, [a], user.id))[0]


@router.get("/track/{ref}", response_model=TrackOut)
async def track(ref: str, user: User = Depends(current_user), s: AsyncSession = Depends(get_session, scope="function")):
    """Metadata only: visible library rows can hydrate without resolving audio."""
    from app.catalog.cache import cache_get
    from app.mixes.personal import hydrate_tracks
    from app.mixes.taste import TasteTrack

    src, ident = _split(ref)
    cached = await cache_get(f"mix:track:{ref}")
    if cached:
        record = CatalogTrack.model_validate(cached)
        if record.ref != ref:
            raise HTTPException(502, "Кэш метаданных содержит другую идентичность")
    elif src == "dz" and svc.deezer:
        try:
            record = await svc.deezer.get_track(ident)
        except Exception as error:
            status = 404 if type(error).__name__ == "CatalogNotFound" else 502
            raise HTTPException(status, "Трек не найден" if status == 404 else "Каталог временно недоступен") from error
    elif src == "hm" and svc.hitmoz_catalog:
        try:
            record = await svc.hitmoz_catalog.get_track(ident)
        except ValueError as error:
            raise HTTPException(400, "Неверная ссылка на трек") from error
        except Exception as error:
            raise HTTPException(502, "Источник метаданных временно недоступен") from error
    elif src == "file":
        from app.listen import file_candidate
        from app.providers.base import file_ref

        candidate = await file_candidate(ref)
        if candidate is None or file_ref(candidate.provider, candidate.provider_id) != ref:
            raise HTTPException(404, "Метаданные файла недоступны")
        if not candidate.cover_url and (provider := svc.providers.get(candidate.provider)):
            try:
                enriched = await provider.enrich(candidate)
                if file_ref(enriched.provider, enriched.provider_id) == ref:
                    candidate = enriched
            except Exception as error:
                raise HTTPException(502, "Источник метаданных временно недоступен") from error
        record = CatalogTrack(ref=ref, source="file", id=ident, title=candidate.title,
                              artist=candidate.artist, duration_s=candidate.duration_s,
                              album_title=candidate.album, cover_url=candidate.cover_url,
                              cover_small_url=candidate.cover_url)
    elif src == "nd":
        member = TasteTrack(key=ref, nd_song_id=ident)
        async with user_navidrome(user) as nd:
            records = await hydrate_tracks([{"members": [member]}], nd)
        record = records[ref]
        if not record.title:
            raise HTTPException(404, "Трек не найден")
    else:
        raise HTTPException(404, "Метаданные трека недоступны")
    result = (await annotate_tracks(s, [record], user.id))[0]
    if src == "nd":
        result.owned, result.nd_song_id = True, ident
    return result
