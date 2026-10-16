from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.media_recording_response import MediaRecordingResponse
from ...models.start_recording_request import StartRecordingRequest
from typing import cast



def _get_kwargs(
    call_id: str,
    *,
    body: StartRecordingRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/calls/{call_id}/recording/start".format(call_id=quote(str(call_id), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
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
    body: StartRecordingRequest,

) -> Response[ErrorResponse | MediaRecordingResponse]:
    """ POST /v1/calls/:call_id/recording/start

     Record a call, through the tenant's consent step
    (`docs/COMPLIANCE_FOLLOWUPS.md` §8.1): the announcement plays to both
    parties and ends before the recorder starts, so this returns once the
    recording is running. `409` when the call is not anchored, is already
    recorded, or the announcement failed under a policy that does not
    record without one.

    Args:
        call_id (str):
        body (StartRecordingRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | MediaRecordingResponse]
     """


    kwargs = _get_kwargs(
        call_id=call_id,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    call_id: str,
    *,
    client: AuthenticatedClient | Client,
    body: StartRecordingRequest,

) -> ErrorResponse | MediaRecordingResponse | None:
    """ POST /v1/calls/:call_id/recording/start

     Record a call, through the tenant's consent step
    (`docs/COMPLIANCE_FOLLOWUPS.md` §8.1): the announcement plays to both
    parties and ends before the recorder starts, so this returns once the
    recording is running. `409` when the call is not anchored, is already
    recorded, or the announcement failed under a policy that does not
    record without one.

    Args:
        call_id (str):
        body (StartRecordingRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | MediaRecordingResponse
     """


    return sync_detailed(
        call_id=call_id,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    call_id: str,
    *,
    client: AuthenticatedClient | Client,
    body: StartRecordingRequest,

) -> Response[ErrorResponse | MediaRecordingResponse]:
    """ POST /v1/calls/:call_id/recording/start

     Record a call, through the tenant's consent step
    (`docs/COMPLIANCE_FOLLOWUPS.md` §8.1): the announcement plays to both
    parties and ends before the recorder starts, so this returns once the
    recording is running. `409` when the call is not anchored, is already
    recorded, or the announcement failed under a policy that does not
    record without one.

    Args:
        call_id (str):
        body (StartRecordingRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | MediaRecordingResponse]
     """


    kwargs = _get_kwargs(
        call_id=call_id,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    call_id: str,
    *,
    client: AuthenticatedClient | Client,
    body: StartRecordingRequest,

) -> ErrorResponse | MediaRecordingResponse | None:
    """ POST /v1/calls/:call_id/recording/start

     Record a call, through the tenant's consent step
    (`docs/COMPLIANCE_FOLLOWUPS.md` §8.1): the announcement plays to both
    parties and ends before the recorder starts, so this returns once the
    recording is running. `409` when the call is not anchored, is already
    recorded, or the announcement failed under a policy that does not
    record without one.

    Args:
        call_id (str):
        body (StartRecordingRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | MediaRecordingResponse
     """


    return (await asyncio_detailed(
        call_id=call_id,
client=client,
body=body,

    )).parsed
