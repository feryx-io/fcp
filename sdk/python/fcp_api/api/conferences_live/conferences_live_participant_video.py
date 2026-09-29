from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from typing import cast



def _get_kwargs(
    room: str,
    pid: str,
    state: str,

) -> dict[str, Any]:
    

    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/conferences/live/{room}/participants/{pid}/video/{state}".format(room=quote(str(room), safe=""),pid=quote(str(pid), safe=""),state=quote(str(state), safe=""),),
    }


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
    state: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[Any | ErrorResponse]:
    """ POST /conferences/live/:room/participants/:pid/video/{on|off}

    Args:
        room (str):
        pid (str):
        state (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        room=room,
pid=pid,
state=state,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    room: str,
    pid: str,
    state: str,
    *,
    client: AuthenticatedClient | Client,

) -> Any | ErrorResponse | None:
    """ POST /conferences/live/:room/participants/:pid/video/{on|off}

    Args:
        room (str):
        pid (str):
        state (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return sync_detailed(
        room=room,
pid=pid,
state=state,
client=client,

    ).parsed

async def asyncio_detailed(
    room: str,
    pid: str,
    state: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[Any | ErrorResponse]:
    """ POST /conferences/live/:room/participants/:pid/video/{on|off}

    Args:
        room (str):
        pid (str):
        state (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        room=room,
pid=pid,
state=state,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    room: str,
    pid: str,
    state: str,
    *,
    client: AuthenticatedClient | Client,

) -> Any | ErrorResponse | None:
    """ POST /conferences/live/:room/participants/:pid/video/{on|off}

    Args:
        room (str):
        pid (str):
        state (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return (await asyncio_detailed(
        room=room,
pid=pid,
state=state,
client=client,

    )).parsed
