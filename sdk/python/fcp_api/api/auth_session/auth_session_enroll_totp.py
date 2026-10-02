from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.mfa_ticket_request import MfaTicketRequest
from ...models.totp_enrollment import TotpEnrollment
from typing import cast



def _get_kwargs(
    *,
    body: MfaTicketRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/auth/totp/enroll",
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | TotpEnrollment:
    if response.status_code == 200:
        response_200 = TotpEnrollment.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | TotpEnrollment]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: MfaTicketRequest,

) -> Response[ErrorResponse | TotpEnrollment]:
    """ POST /api/v1/auth/totp/enroll — set up the second factor a tenant
    requires, in the middle of signing in.

    Args:
        body (MfaTicketRequest): A ticket to set a second factor up with.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | TotpEnrollment]
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
    body: MfaTicketRequest,

) -> ErrorResponse | TotpEnrollment | None:
    """ POST /api/v1/auth/totp/enroll — set up the second factor a tenant
    requires, in the middle of signing in.

    Args:
        body (MfaTicketRequest): A ticket to set a second factor up with.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | TotpEnrollment
     """


    return sync_detailed(
        client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: MfaTicketRequest,

) -> Response[ErrorResponse | TotpEnrollment]:
    """ POST /api/v1/auth/totp/enroll — set up the second factor a tenant
    requires, in the middle of signing in.

    Args:
        body (MfaTicketRequest): A ticket to set a second factor up with.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | TotpEnrollment]
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
    body: MfaTicketRequest,

) -> ErrorResponse | TotpEnrollment | None:
    """ POST /api/v1/auth/totp/enroll — set up the second factor a tenant
    requires, in the middle of signing in.

    Args:
        body (MfaTicketRequest): A ticket to set a second factor up with.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | TotpEnrollment
     """


    return (await asyncio_detailed(
        client=client,
body=body,

    )).parsed
