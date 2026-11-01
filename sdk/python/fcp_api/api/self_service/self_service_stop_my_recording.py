from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.media_recording_response import MediaRecordingResponse
from typing import cast



def _get_kwargs(
    call_id: str,

) -> dict[str, Any]:
    

    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/me/calls/{call_id}/recording/stop".format(call_id=quote(str(call_id), safe=""),),
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | MediaRecordingResponse:
    if response.status_code == 200:
        response_200 = MediaRecordingResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | MediaRecordingResponse]:
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

) -> Response[ErrorResponse | MediaRecordingResponse]:
    """ POST /me/calls/:call_id/recording/stop — stop a recording the user
    started on demand. Any other recording of the call goes on (409).

    Args:
        call_id (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | MediaRecordingResponse]
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

) -> ErrorResponse | MediaRecordingResponse | None:
    """ POST /me/calls/:call_id/recording/stop — stop a recording the user
    started on demand. Any other recording of the call goes on (409).

    Args:
        call_id (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | MediaRecordingResponse
     """


    return sync_detailed(
        call_id=call_id,
client=client,

    ).parsed

async def asyncio_detailed(
    call_id: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[ErrorResponse | MediaRecordingResponse]:
    """ POST /me/calls/:call_id/recording/stop — stop a recording the user
    started on demand. Any other recording of the call goes on (409).

    Args:
        call_id (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | MediaRecordingResponse]
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

) -> ErrorResponse | MediaRecordingResponse | None:
    """ POST /me/calls/:call_id/recording/stop — stop a recording the user
    started on demand. Any other recording of the call goes on (409).

    Args:
        call_id (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | MediaRecordingResponse
     """


    return (await asyncio_detailed(
        call_id=call_id,
client=client,

    )).parsed
