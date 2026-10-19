"""Small system integrations; metadata contains no persistent device identifier."""
from ._device import request, exchange


def info():
    return exchange({"op": "info"})


async def battery():
    return await request("system.battery")


async def network():
    return await request("system.network")


async def open_url(url):
    from urllib.parse import urlsplit
    if urlsplit(url).scheme not in ("https", "http", "mailto", "tel", "geo"):
        raise ValueError("Supported schemes: https, http, mailto, tel, geo")
    await request("system.open", url=url)


async def keep_awake(enabled=True):
    await request("system.awake", enabled=bool(enabled))
