from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.page_subject_request_response import PageSubjectRequestResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    status: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["status"] = status

    params["limit"] = limit

    params["cursor"] = cursor


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/compliance/subject-requests",
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | PageSubjectRequestResponse:
    if response.status_code == 200:
        response_200 = PageSubjectRequestResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | PageSubjectRequestResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    status: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> Response[ErrorResponse | PageSubjectRequestResponse]:
    """ `GET /compliance/subject-requests`: the tenant's requests, newest first.

    Args:
        status (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageSubjectRequestResponse]
     """


    kwargs = _get_kwargs(
        status=status,
limit=limit,
cursor=cursor,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    status: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> ErrorResponse | PageSubjectRequestResponse | None:
    """ `GET /compliance/subject-requests`: the tenant's requests, newest first.

    Args:
        status (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageSubjectRequestResponse
     """


    return sync_detailed(
        client=client,
status=status,
limit=limit,
cursor=cursor,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    status: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> Response[ErrorResponse | PageSubjectRequestResponse]:
    """ `GET /compliance/subject-requests`: the tenant's requests, newest first.

    Args:
        status (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageSubjectRequestResponse]
     """


    kwargs = _get_kwargs(
        status=status,
limit=limit,
cursor=cursor,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    status: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> ErrorResponse | PageSubjectRequestResponse | None:
    """ `GET /compliance/subject-requests`: the tenant's requests, newest first.

    Args:
        status (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageSubjectRequestResponse
     """


    return (await asyncio_detailed(
        client=client,
status=status,
limit=limit,
cursor=cursor,

    )).parsed
