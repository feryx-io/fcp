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
    *,
    body: Any,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/conferences/live/{room}/dial-out".format(room=quote(str(room), safe=""),),
    }

    _kwargs["json"] = body

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
    *,
    client: AuthenticatedClient | Client,
    body: Any,

) -> Response[Any | ErrorResponse]:
    """ POST /conferences/live/:room/dial-out — body passed through
    (`destination`, optional `greeting_tts` / `greeting_prompt`).

    Args:
        room (str):
        body (Any):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        room=room,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    room: str,
    *,
    client: AuthenticatedClient | Client,
    body: Any,

) -> Any | ErrorResponse | None:
    """ POST /conferences/live/:room/dial-out — body passed through
    (`destination`, optional `greeting_tts` / `greeting_prompt`).

    Args:
        room (str):
        body (Any):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return sync_detailed(
        room=room,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    room: str,
    *,
    client: AuthenticatedClient | Client,
    body: Any,

) -> Response[Any | ErrorResponse]:
    """ POST /conferences/live/:room/dial-out — body passed through
    (`destination`, optional `greeting_tts` / `greeting_prompt`).

    Args:
        room (str):
        body (Any):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        room=room,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    room: str,
    *,
    client: AuthenticatedClient | Client,
    body: Any,

) -> Any | ErrorResponse | None:
    """ POST /conferences/live/:room/dial-out — body passed through
    (`destination`, optional `greeting_tts` / `greeting_prompt`).

    Args:
        room (str):
        body (Any):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return (await asyncio_detailed(
        room=room,
client=client,
body=body,

    )).parsed
