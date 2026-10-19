"""Permission inspection; protected operations also request permission just in time."""
from ._device import request


async def status(permission):
    return await request("permissions.status", permission=permission)


async def ask(permission):
    return await request("permissions.ask", permission=permission)


async def open_settings():
    await request("system.settings")
