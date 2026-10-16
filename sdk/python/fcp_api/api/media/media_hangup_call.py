from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.call_control_response import CallControlResponse
from ...models.error_response import ErrorResponse
from typing import cast



def _get_kwargs(
    call_id: str,

) -> dict[str, Any]:
    

    

    

    _kwargs: dict[str, Any] = {
        "method": "delete",
        "url": "/api/v1/calls/{call_id}".format(call_id=quote(str(call_id), safe=""),),
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> CallControlResponse | ErrorResponse:
    if response.status_code == 200:
        response_200 = CallControlResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[CallControlResponse | ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    call_id: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[CallControlResponse | ErrorResponse]:
    """ DELETE /v1/calls/:call_id
    Hang up a call: `POST /calls/:call_id/hangup` by another name.

    Args:
        call_id (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[CallControlResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        call_id=call_id,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    call_id: str,
    *,
    client: AuthenticatedClient | Client,

) -> CallControlResponse | ErrorResponse | None:
    """ DELETE /v1/calls/:call_id
    Hang up a call: `POST /calls/:call_id/hangup` by another name.

    Args:
        call_id (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        CallControlResponse | ErrorResponse
     """


    return sync_detailed(
        call_id=call_id,
client=client,

    ).parsed

async def asyncio_detailed(
    call_id: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[CallControlResponse | ErrorResponse]:
    """ DELETE /v1/calls/:call_id
    Hang up a call: `POST /calls/:call_id/hangup` by another name.

    Args:
        call_id (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[CallControlResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        call_id=call_id,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    call_id: str,
    *,
    client: AuthenticatedClient | Client,

) -> CallControlResponse | ErrorResponse | None:
    """ DELETE /v1/calls/:call_id
    Hang up a call: `POST /calls/:call_id/hangup` by another name.

    Args:
        call_id (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        CallControlResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        call_id=call_id,
client=client,

    )).parsed
