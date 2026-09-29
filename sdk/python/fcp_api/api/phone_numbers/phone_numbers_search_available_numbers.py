from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.page_phone_number_response import PagePhoneNumberResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    area_code: str | Unset = UNSET,
    pattern: str | Unset = UNSET,
    rate_center: str | Unset = UNSET,
    limit: int | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["area_code"] = area_code

    params["pattern"] = pattern

    params["rate_center"] = rate_center

    params["limit"] = limit


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/phone-numbers/available",
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | PagePhoneNumberResponse:
    if response.status_code == 200:
        response_200 = PagePhoneNumberResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | PagePhoneNumberResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    area_code: str | Unset = UNSET,
    pattern: str | Unset = UNSET,
    rate_center: str | Unset = UNSET,
    limit: int | Unset = UNSET,

) -> Response[ErrorResponse | PagePhoneNumberResponse]:
    """ Search available pool phone numbers.
    GET /phone-numbers/available?area_code=518&pattern=*7034&limit=20

    Args:
        area_code (str | Unset):
        pattern (str | Unset):
        rate_center (str | Unset):
        limit (int | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PagePhoneNumberResponse]
     """


    kwargs = _get_kwargs(
        area_code=area_code,
pattern=pattern,
rate_center=rate_center,
limit=limit,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    area_code: str | Unset = UNSET,
    pattern: str | Unset = UNSET,
    rate_center: str | Unset = UNSET,
    limit: int | Unset = UNSET,

) -> ErrorResponse | PagePhoneNumberResponse | None:
    """ Search available pool phone numbers.
    GET /phone-numbers/available?area_code=518&pattern=*7034&limit=20

    Args:
        area_code (str | Unset):
        pattern (str | Unset):
        rate_center (str | Unset):
        limit (int | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PagePhoneNumberResponse
     """


    return sync_detailed(
        client=client,
area_code=area_code,
pattern=pattern,
rate_center=rate_center,
limit=limit,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    area_code: str | Unset = UNSET,
    pattern: str | Unset = UNSET,
    rate_center: str | Unset = UNSET,
    limit: int | Unset = UNSET,

) -> Response[ErrorResponse | PagePhoneNumberResponse]:
    """ Search available pool phone numbers.
    GET /phone-numbers/available?area_code=518&pattern=*7034&limit=20

    Args:
        area_code (str | Unset):
        pattern (str | Unset):
        rate_center (str | Unset):
        limit (int | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PagePhoneNumberResponse]
     """


    kwargs = _get_kwargs(
        area_code=area_code,
pattern=pattern,
rate_center=rate_center,
limit=limit,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    area_code: str | Unset = UNSET,
    pattern: str | Unset = UNSET,
    rate_center: str | Unset = UNSET,
    limit: int | Unset = UNSET,

) -> ErrorResponse | PagePhoneNumberResponse | None:
    """ Search available pool phone numbers.
    GET /phone-numbers/available?area_code=518&pattern=*7034&limit=20

    Args:
        area_code (str | Unset):
        pattern (str | Unset):
        rate_center (str | Unset):
        limit (int | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PagePhoneNumberResponse
     """


    return (await asyncio_detailed(
        client=client,
area_code=area_code,
pattern=pattern,
rate_center=rate_center,
limit=limit,

    )).parsed
