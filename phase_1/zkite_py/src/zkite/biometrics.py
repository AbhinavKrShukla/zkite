"""System biometric prompt (API 28+). Does not cryptographically gate secrets."""
from ._device import request


async def authenticate(*, title="Verify your identity"):
    return await request("biometrics.authenticate", title=title)
