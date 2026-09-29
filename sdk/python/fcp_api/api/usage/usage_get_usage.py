from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.usage_response import UsageResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    period: str | Unset = UNSET,
    from_: str | Unset = UNSET,
    to: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["period"] = period

    params["from"] = from_

    params["to"] = to


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/usage",
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | UsageResponse:
    if response.status_code == 200:
        response_200 = UsageResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | UsageResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    period: str | Unset = UNSET,
    from_: str | Unset = UNSET,
    to: str | Unset = UNSET,

) -> Response[ErrorResponse | UsageResponse]:
    """ GET /api/v1/usage — Aggregate CDR totals for a tenant.

    Args:
        period (str | Unset):
        from_ (str | Unset):
        to (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | UsageResponse]
     """


    kwargs = _get_kwargs(
        period=period,
from_=from_,
to=to,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    period: str | Unset = UNSET,
    from_: str | Unset = UNSET,
    to: str | Unset = UNSET,

) -> ErrorResponse | UsageResponse | None:
    """ GET /api/v1/usage — Aggregate CDR totals for a tenant.

    Args:
        period (str | Unset):
        from_ (str | Unset):
        to (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | UsageResponse
     """


    return sync_detailed(
        client=client,
period=period,
from_=from_,
to=to,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    period: str | Unset = UNSET,
    from_: str | Unset = UNSET,
    to: str | Unset = UNSET,

) -> Response[ErrorResponse | UsageResponse]:
    """ GET /api/v1/usage — Aggregate CDR totals for a tenant.

    Args:
        period (str | Unset):
        from_ (str | Unset):
        to (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | UsageResponse]
     """


    kwargs = _get_kwargs(
        period=period,
from_=from_,
to=to,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    period: str | Unset = UNSET,
    from_: str | Unset = UNSET,
    to: str | Unset = UNSET,

) -> ErrorResponse | UsageResponse | None:
    """ GET /api/v1/usage — Aggregate CDR totals for a tenant.

    Args:
        period (str | Unset):
        from_ (str | Unset):
        to (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | UsageResponse
     """


    return (await asyncio_detailed(
        client=client,
period=period,
from_=from_,
to=to,

    )).parsed
