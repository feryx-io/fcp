from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.classify_response import ClassifyResponse
from ...models.error_response import ErrorResponse
from typing import cast



def _get_kwargs(
    *,
    dialed: str,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["dialed"] = dialed


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/classes-of-service/classify",
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ClassifyResponse | ErrorResponse:
    if response.status_code == 200:
        response_200 = ClassifyResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ClassifyResponse | ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    dialed: str,

) -> Response[ClassifyResponse | ErrorResponse]:
    """ GET /api/v1/classes-of-service/classify?dialed=… — what a number is to
    the tenant, by its country and overrides. Extensions are read as the
    platform's default of six digits.

    Args:
        dialed (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ClassifyResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        dialed=dialed,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    dialed: str,

) -> ClassifyResponse | ErrorResponse | None:
    """ GET /api/v1/classes-of-service/classify?dialed=… — what a number is to
    the tenant, by its country and overrides. Extensions are read as the
    platform's default of six digits.

    Args:
        dialed (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ClassifyResponse | ErrorResponse
     """


    return sync_detailed(
        client=client,
dialed=dialed,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    dialed: str,

) -> Response[ClassifyResponse | ErrorResponse]:
    """ GET /api/v1/classes-of-service/classify?dialed=… — what a number is to
    the tenant, by its country and overrides. Extensions are read as the
    platform's default of six digits.

    Args:
        dialed (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ClassifyResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        dialed=dialed,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    dialed: str,

) -> ClassifyResponse | ErrorResponse | None:
    """ GET /api/v1/classes-of-service/classify?dialed=… — what a number is to
    the tenant, by its country and overrides. Extensions are read as the
    platform's default of six digits.

    Args:
        dialed (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ClassifyResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        client=client,
dialed=dialed,

    )).parsed
