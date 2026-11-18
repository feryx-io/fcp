from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    room: str,
    *,
    call_id: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["call_id"] = call_id


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/me/conferences/live/{room}/questions".format(room=quote(str(room), safe=""),),
        "params": params,
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
    *,
    client: AuthenticatedClient | Client,
    call_id: str | Unset = UNSET,

) -> Response[Any | ErrorResponse]:
    """ GET /me/conferences/live/:room/questions — the room's questions
    (`docs/WEBINARS.md` §2.5): as the participant on `call_id` sees them
    (any user of the room's tenant, the call naming whose view it is), or
    every one for a host or panelist.

    Args:
        room (str):
        call_id (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        room=room,
call_id=call_id,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    room: str,
    *,
    client: AuthenticatedClient | Client,
    call_id: str | Unset = UNSET,

) -> Any | ErrorResponse | None:
    """ GET /me/conferences/live/:room/questions — the room's questions
    (`docs/WEBINARS.md` §2.5): as the participant on `call_id` sees them
    (any user of the room's tenant, the call naming whose view it is), or
    every one for a host or panelist.

    Args:
        room (str):
        call_id (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return sync_detailed(
        room=room,
client=client,
call_id=call_id,

    ).parsed

async def asyncio_detailed(
    room: str,
    *,
    client: AuthenticatedClient | Client,
    call_id: str | Unset = UNSET,

) -> Response[Any | ErrorResponse]:
    """ GET /me/conferences/live/:room/questions — the room's questions
    (`docs/WEBINARS.md` §2.5): as the participant on `call_id` sees them
    (any user of the room's tenant, the call naming whose view it is), or
    every one for a host or panelist.

    Args:
        room (str):
        call_id (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        room=room,
call_id=call_id,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    room: str,
    *,
    client: AuthenticatedClient | Client,
    call_id: str | Unset = UNSET,

) -> Any | ErrorResponse | None:
    """ GET /me/conferences/live/:room/questions — the room's questions
    (`docs/WEBINARS.md` §2.5): as the participant on `call_id` sees them
    (any user of the room's tenant, the call naming whose view it is), or
    every one for a host or panelist.

    Args:
        room (str):
        call_id (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return (await asyncio_detailed(
        room=room,
client=client,
call_id=call_id,

    )).parsed
