from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.config_entry_response import ConfigEntryResponse
from ...models.error_response import ErrorResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    service: str,
    section: str,
    *,
    tenant_id: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["tenant_id"] = tenant_id


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/config/{service}/{section}".format(service=quote(str(service), safe=""),section=quote(str(section), safe=""),),
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ConfigEntryResponse | ErrorResponse:
    if response.status_code == 200:
        response_200 = ConfigEntryResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ConfigEntryResponse | ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    service: str,
    section: str,
    *,
    client: AuthenticatedClient | Client,
    tenant_id: str | Unset = UNSET,

) -> Response[ConfigEntryResponse | ErrorResponse]:
    """ GET /api/v1/config/:service/:section -- get specific section

    Args:
        service (str):
        section (str):
        tenant_id (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ConfigEntryResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        service=service,
section=section,
tenant_id=tenant_id,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    service: str,
    section: str,
    *,
    client: AuthenticatedClient | Client,
    tenant_id: str | Unset = UNSET,

) -> ConfigEntryResponse | ErrorResponse | None:
    """ GET /api/v1/config/:service/:section -- get specific section

    Args:
        service (str):
        section (str):
        tenant_id (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ConfigEntryResponse | ErrorResponse
     """


    return sync_detailed(
        service=service,
section=section,
client=client,
tenant_id=tenant_id,

    ).parsed

async def asyncio_detailed(
    service: str,
    section: str,
    *,
    client: AuthenticatedClient | Client,
    tenant_id: str | Unset = UNSET,

) -> Response[ConfigEntryResponse | ErrorResponse]:
    """ GET /api/v1/config/:service/:section -- get specific section

    Args:
        service (str):
        section (str):
        tenant_id (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ConfigEntryResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        service=service,
section=section,
tenant_id=tenant_id,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    service: str,
    section: str,
    *,
    client: AuthenticatedClient | Client,
    tenant_id: str | Unset = UNSET,

) -> ConfigEntryResponse | ErrorResponse | None:
    """ GET /api/v1/config/:service/:section -- get specific section

    Args:
        service (str):
        section (str):
        tenant_id (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ConfigEntryResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        service=service,
section=section,
client=client,
tenant_id=tenant_id,

    )).parsed
