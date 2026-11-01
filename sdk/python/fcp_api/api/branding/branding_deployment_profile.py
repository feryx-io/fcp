from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    format_: str | Unset = UNSET,
    sign_in: str | Unset = UNSET,
    updates: str | Unset = UNSET,
    lock_server: bool | Unset = UNSET,
    username_from_system: bool | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["format"] = format_

    params["sign_in"] = sign_in

    params["updates"] = updates

    params["lock_server"] = lock_server

    params["username_from_system"] = username_from_system


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/branding/deployment-profile",
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse:
    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    format_: str | Unset = UNSET,
    sign_in: str | Unset = UNSET,
    updates: str | Unset = UNSET,
    lock_server: bool | Unset = UNSET,
    username_from_system: bool | Unset = UNSET,

) -> Response[ErrorResponse]:
    """ GET /api/v1/branding/deployment-profile — the tenant's softphone profile
    (Anvil's `docs/MANAGED_INSTALLS.md` §2.1): this admin's address as the
    server, the tenant's first login domain, its sign-in, and its brand with
    the files, for `Anvil-…-setup.exe /PROFILE=` or Windows policy.

    Args:
        format_ (str | Unset):
        sign_in (str | Unset):
        updates (str | Unset):
        lock_server (bool | Unset):
        username_from_system (bool | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse]
     """


    kwargs = _get_kwargs(
        format_=format_,
sign_in=sign_in,
updates=updates,
lock_server=lock_server,
username_from_system=username_from_system,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    format_: str | Unset = UNSET,
    sign_in: str | Unset = UNSET,
    updates: str | Unset = UNSET,
    lock_server: bool | Unset = UNSET,
    username_from_system: bool | Unset = UNSET,

) -> ErrorResponse | None:
    """ GET /api/v1/branding/deployment-profile — the tenant's softphone profile
    (Anvil's `docs/MANAGED_INSTALLS.md` §2.1): this admin's address as the
    server, the tenant's first login domain, its sign-in, and its brand with
    the files, for `Anvil-…-setup.exe /PROFILE=` or Windows policy.

    Args:
        format_ (str | Unset):
        sign_in (str | Unset):
        updates (str | Unset):
        lock_server (bool | Unset):
        username_from_system (bool | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse
     """


    return sync_detailed(
        client=client,
format_=format_,
sign_in=sign_in,
updates=updates,
lock_server=lock_server,
username_from_system=username_from_system,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    format_: str | Unset = UNSET,
    sign_in: str | Unset = UNSET,
    updates: str | Unset = UNSET,
    lock_server: bool | Unset = UNSET,
    username_from_system: bool | Unset = UNSET,

) -> Response[ErrorResponse]:
    """ GET /api/v1/branding/deployment-profile — the tenant's softphone profile
    (Anvil's `docs/MANAGED_INSTALLS.md` §2.1): this admin's address as the
    server, the tenant's first login domain, its sign-in, and its brand with
    the files, for `Anvil-…-setup.exe /PROFILE=` or Windows policy.

    Args:
        format_ (str | Unset):
        sign_in (str | Unset):
        updates (str | Unset):
        lock_server (bool | Unset):
        username_from_system (bool | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse]
     """


    kwargs = _get_kwargs(
        format_=format_,
sign_in=sign_in,
updates=updates,
lock_server=lock_server,
username_from_system=username_from_system,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    format_: str | Unset = UNSET,
    sign_in: str | Unset = UNSET,
    updates: str | Unset = UNSET,
    lock_server: bool | Unset = UNSET,
    username_from_system: bool | Unset = UNSET,

) -> ErrorResponse | None:
    """ GET /api/v1/branding/deployment-profile — the tenant's softphone profile
    (Anvil's `docs/MANAGED_INSTALLS.md` §2.1): this admin's address as the
    server, the tenant's first login domain, its sign-in, and its brand with
    the files, for `Anvil-…-setup.exe /PROFILE=` or Windows policy.

    Args:
        format_ (str | Unset):
        sign_in (str | Unset):
        updates (str | Unset):
        lock_server (bool | Unset):
        username_from_system (bool | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse
     """


    return (await asyncio_detailed(
        client=client,
format_=format_,
sign_in=sign_in,
updates=updates,
lock_server=lock_server,
username_from_system=username_from_system,

    )).parsed
