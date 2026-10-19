"""Small encrypted values using an app-owned Android Keystore key.

Storage is excluded from Android backup. Losing the key makes ciphertext unreadable.
Biometric authentication is a separate UI operation, not an encryption guarantee.
"""
from ._device import request


def _key(name):
    if not isinstance(name, str) or not 1 <= len(name) <= 200:
        raise ValueError("Secret name must contain 1–200 characters")
    return name


async def set(name, value):
    if not isinstance(value, str) or len(value.encode()) > 16384:
        raise ValueError("Secret must be a string of at most 16 KiB")
    await request("secrets.set", name=_key(name), value=value)


async def get(name):
    return await request("secrets.get", name=_key(name))


async def delete(name):
    await request("secrets.delete", name=_key(name))
