from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.page_speech_usage_period import PageSpeechUsagePeriod
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    from_: str | Unset = UNSET,
    to: str | Unset = UNSET,
    by: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["from"] = from_

    params["to"] = to

    params["by"] = by


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/speech/usage",
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | PageSpeechUsagePeriod:
    if response.status_code == 200:
        response_200 = PageSpeechUsagePeriod.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | PageSpeechUsagePeriod]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    from_: str | Unset = UNSET,
    to: str | Unset = UNSET,
    by: str | Unset = UNSET,

) -> Response[ErrorResponse | PageSpeechUsagePeriod]:
    """ GET /speech/usage — the tenant's speech by month or day and provider.

    Args:
        from_ (str | Unset):
        to (str | Unset):
        by (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageSpeechUsagePeriod]
     """


    kwargs = _get_kwargs(
        from_=from_,
to=to,
by=by,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    from_: str | Unset = UNSET,
    to: str | Unset = UNSET,
    by: str | Unset = UNSET,

) -> ErrorResponse | PageSpeechUsagePeriod | None:
    """ GET /speech/usage — the tenant's speech by month or day and provider.

    Args:
        from_ (str | Unset):
        to (str | Unset):
        by (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageSpeechUsagePeriod
     """


    return sync_detailed(
        client=client,
from_=from_,
to=to,
by=by,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    from_: str | Unset = UNSET,
    to: str | Unset = UNSET,
    by: str | Unset = UNSET,

) -> Response[ErrorResponse | PageSpeechUsagePeriod]:
    """ GET /speech/usage — the tenant's speech by month or day and provider.

    Args:
        from_ (str | Unset):
        to (str | Unset):
        by (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageSpeechUsagePeriod]
     """


    kwargs = _get_kwargs(
        from_=from_,
to=to,
by=by,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    from_: str | Unset = UNSET,
    to: str | Unset = UNSET,
    by: str | Unset = UNSET,

) -> ErrorResponse | PageSpeechUsagePeriod | None:
    """ GET /speech/usage — the tenant's speech by month or day and provider.

    Args:
        from_ (str | Unset):
        to (str | Unset):
        by (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageSpeechUsagePeriod
     """


    return (await asyncio_detailed(
        client=client,
from_=from_,
to=to,
by=by,

    )).parsed
