import httpx
import pytest

from app.net import request_with_backoff


@pytest.mark.parametrize("proxied_result", [200, 404, 403, 407, 502, None])
async def test_catalog_fallback_is_used_only_after_proxy_failure(proxied_result):
    seen = []

    def proxied(request):
        seen.append("proxy")
        if proxied_result is None:
            raise httpx.ConnectTimeout("Proxy timeout", request=request)
        return httpx.Response(proxied_result)

    def direct(request):
        seen.append("direct")
        assert request.url.params["q"] == "Radiohead"
        return httpx.Response(200)

    async with (httpx.AsyncClient(transport=httpx.MockTransport(proxied)) as proxy,
                httpx.AsyncClient(transport=httpx.MockTransport(direct), trust_env=False) as fallback):
        response = await request_with_backoff(proxy, "GET", "https://catalog.invalid/search",
                                              direct_fallback=fallback, params={"q": "Radiohead"})

    if proxied_result in {200, 404}:
        assert seen == ["proxy"]
        assert response.status_code == proxied_result
    else:
        assert seen == ["proxy", "direct"]
        assert response.status_code == 200
