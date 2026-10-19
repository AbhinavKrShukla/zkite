"""Immediate local notifications. No background scheduler is implied."""
from ._device import request


async def show(title, body="", *, id=1, channel="general", channel_name="General"):
    if type(id) is not int or not 0 <= id <= 2147483647:
        raise ValueError("id must be a nonnegative 32-bit integer")
    await request("notifications.show", title=str(title), body=str(body), notification_id=id,
                  channel=channel, channel_name=channel_name)


async def cancel(id=1):
    await request("notifications.cancel", notification_id=id)
