from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.call_quality_report import CallQualityReport
from ...models.call_quality_response import CallQualityResponse
from ...models.error_response import ErrorResponse
from typing import cast



def _get_kwargs(
    call_id: str,
    *,
    body: CallQualityReport,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/me/calls/{call_id}/quality".format(call_id=quote(str(call_id), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> CallQualityResponse | ErrorResponse:
    if response.status_code == 201:
        response_201 = CallQualityResponse.from_dict(response.json())



        return response_201

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[CallQualityResponse | ErrorResponse]:
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
    body: CallQualityReport,

) -> Response[CallQualityResponse | ErrorResponse]:
    """ POST /me/calls/:call_id/quality — the app's final figures for a call
    it was on, sent when the call ends. A second report from the same app
    replaces its first. 404 when the call's record names other people (one
    not yet written is taken on the app's word); `call.quality_poor` when
    either score is under 3.1.

    Args:
        call_id (str):
        body (CallQualityReport): A call's final figures, as the app measured them.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[CallQualityResponse | ErrorResponse]
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
    body: CallQualityReport,

) -> CallQualityResponse | ErrorResponse | None:
    """ POST /me/calls/:call_id/quality — the app's final figures for a call
    it was on, sent when the call ends. A second report from the same app
    replaces its first. 404 when the call's record names other people (one
    not yet written is taken on the app's word); `call.quality_poor` when
    either score is under 3.1.

    Args:
        call_id (str):
        body (CallQualityReport): A call's final figures, as the app measured them.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        CallQualityResponse | ErrorResponse
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
    body: CallQualityReport,

) -> Response[CallQualityResponse | ErrorResponse]:
    """ POST /me/calls/:call_id/quality — the app's final figures for a call
    it was on, sent when the call ends. A second report from the same app
    replaces its first. 404 when the call's record names other people (one
    not yet written is taken on the app's word); `call.quality_poor` when
    either score is under 3.1.

    Args:
        call_id (str):
        body (CallQualityReport): A call's final figures, as the app measured them.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[CallQualityResponse | ErrorResponse]
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
    body: CallQualityReport,

) -> CallQualityResponse | ErrorResponse | None:
    """ POST /me/calls/:call_id/quality — the app's final figures for a call
    it was on, sent when the call ends. A second report from the same app
    replaces its first. 404 when the call's record names other people (one
    not yet written is taken on the app's word); `call.quality_poor` when
    either score is under 3.1.

    Args:
        call_id (str):
        body (CallQualityReport): A call's final figures, as the app measured them.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        CallQualityResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        call_id=call_id,
client=client,
body=body,

    )).parsed
