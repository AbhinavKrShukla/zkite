from ._device import request


async def write(text):
    await request("clipboard.write", text=str(text))


async def read():
    return await request("clipboard.read")
