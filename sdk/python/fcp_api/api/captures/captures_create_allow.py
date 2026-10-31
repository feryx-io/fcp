from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.capture_allow_request import CaptureAllowRequest
from ...models.capture_allow_response import CaptureAllowResponse
from ...models.error_response import ErrorResponse
from typing import cast



def _get_kwargs(
    *,
    body: CaptureAllowRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/captures/allows",
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> CaptureAllowResponse | ErrorResponse:
    if response.status_code == 201:
        response_201 = CaptureAllowResponse.from_dict(response.json())



        return response_201

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[CaptureAllowResponse | ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: CaptureAllowRequest,

) -> Response[CaptureAllowResponse | ErrorResponse]:
    """ POST /captures/allows — allow a user or a device to capture until a
    date. 409 while the tenant has captures off.

    Args:
        body (CaptureAllowRequest): Allow a user (every app they sign in to) or one device to
            capture.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[CaptureAllowResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    body: CaptureAllowRequest,

) -> CaptureAllowResponse | ErrorResponse | None:
    """ POST /captures/allows — allow a user or a device to capture until a
    date. 409 while the tenant has captures off.

    Args:
        body (CaptureAllowRequest): Allow a user (every app they sign in to) or one device to
            capture.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        CaptureAllowResponse | ErrorResponse
     """


    return sync_detailed(
        client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: CaptureAllowRequest,

) -> Response[CaptureAllowResponse | ErrorResponse]:
    """ POST /captures/allows — allow a user or a device to capture until a
    date. 409 while the tenant has captures off.

    Args:
        body (CaptureAllowRequest): Allow a user (every app they sign in to) or one device to
            capture.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[CaptureAllowResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    body: CaptureAllowRequest,

) -> CaptureAllowResponse | ErrorResponse | None:
    """ POST /captures/allows — allow a user or a device to capture until a
    date. 409 while the tenant has captures off.

    Args:
        body (CaptureAllowRequest): Allow a user (every app they sign in to) or one device to
            capture.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        CaptureAllowResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        client=client,
body=body,

    )).parsed
