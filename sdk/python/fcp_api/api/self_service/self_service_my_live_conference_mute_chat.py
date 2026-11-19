from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.mute_chat_request import MuteChatRequest
from typing import cast



def _get_kwargs(
    room: str,
    pid: str,
    *,
    body: MuteChatRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/me/conferences/live/{room}/participants/{pid}/chat-mute".format(room=quote(str(room), safe=""),pid=quote(str(pid), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Any | ErrorResponse:
    if response.status_code == 200:
        response_200 = response.json()
        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[Any | ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    room: str,
    pid: str,
    *,
    client: AuthenticatedClient | Client,
    body: MuteChatRequest,

) -> Response[Any | ErrorResponse]:
    """ POST /me/conferences/live/:room/participants/:pid/chat-mute — a host or
    panelist mutes a participant's chat for the rest of the sitting, or
    unmutes it. A host or panelist is not muted.

    Args:
        room (str):
        pid (str):
        body (MuteChatRequest): The body of `POST
            /me/conferences/live/:room/participants/:pid/chat-mute`.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        room=room,
pid=pid,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    room: str,
    pid: str,
    *,
    client: AuthenticatedClient | Client,
    body: MuteChatRequest,

) -> Any | ErrorResponse | None:
    """ POST /me/conferences/live/:room/participants/:pid/chat-mute — a host or
    panelist mutes a participant's chat for the rest of the sitting, or
    unmutes it. A host or panelist is not muted.

    Args:
        room (str):
        pid (str):
        body (MuteChatRequest): The body of `POST
            /me/conferences/live/:room/participants/:pid/chat-mute`.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return sync_detailed(
        room=room,
pid=pid,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    room: str,
    pid: str,
    *,
    client: AuthenticatedClient | Client,
    body: MuteChatRequest,

) -> Response[Any | ErrorResponse]:
    """ POST /me/conferences/live/:room/participants/:pid/chat-mute — a host or
    panelist mutes a participant's chat for the rest of the sitting, or
    unmutes it. A host or panelist is not muted.

    Args:
        room (str):
        pid (str):
        body (MuteChatRequest): The body of `POST
            /me/conferences/live/:room/participants/:pid/chat-mute`.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        room=room,
pid=pid,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    room: str,
    pid: str,
    *,
    client: AuthenticatedClient | Client,
    body: MuteChatRequest,

) -> Any | ErrorResponse | None:
    """ POST /me/conferences/live/:room/participants/:pid/chat-mute — a host or
    panelist mutes a participant's chat for the rest of the sitting, or
    unmutes it. A host or panelist is not muted.

    Args:
        room (str):
        pid (str):
        body (MuteChatRequest): The body of `POST
            /me/conferences/live/:room/participants/:pid/chat-mute`.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return (await asyncio_detailed(
        room=room,
pid=pid,
client=client,
body=body,

    )).parsed
