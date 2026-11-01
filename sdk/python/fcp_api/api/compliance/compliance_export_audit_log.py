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
    actor: str | Unset = UNSET,
    action: str | Unset = UNSET,
    since: int | Unset = UNSET,
    until: int | Unset = UNSET,
    tenant: str | Unset = UNSET,
    format_: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["actor"] = actor

    params["action"] = action

    params["since"] = since

    params["until"] = until

    params["tenant"] = tenant

    params["format"] = format_


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/audit-log/export",
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | str:
    if response.status_code == 200:
        response_200 = response.text
        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | str]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    actor: str | Unset = UNSET,
    action: str | Unset = UNSET,
    since: int | Unset = UNSET,
    until: int | Unset = UNSET,
    tenant: str | Unset = UNSET,
    format_: str | Unset = UNSET,

) -> Response[ErrorResponse | str]:
    """ `GET /audit-log/export`: every event the listing's filters match,
    newest first, streamed page by page as CSV or JSON Lines (D8, finding
    36). Who may see what is the listing's.

    Args:
        actor (str | Unset):
        action (str | Unset):
        since (int | Unset):
        until (int | Unset):
        tenant (str | Unset):
        format_ (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | str]
     """


    kwargs = _get_kwargs(
        actor=actor,
action=action,
since=since,
until=until,
tenant=tenant,
format_=format_,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    actor: str | Unset = UNSET,
    action: str | Unset = UNSET,
    since: int | Unset = UNSET,
    until: int | Unset = UNSET,
    tenant: str | Unset = UNSET,
    format_: str | Unset = UNSET,

) -> ErrorResponse | str | None:
    """ `GET /audit-log/export`: every event the listing's filters match,
    newest first, streamed page by page as CSV or JSON Lines (D8, finding
    36). Who may see what is the listing's.

    Args:
        actor (str | Unset):
        action (str | Unset):
        since (int | Unset):
        until (int | Unset):
        tenant (str | Unset):
        format_ (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | str
     """


    return sync_detailed(
        client=client,
actor=actor,
action=action,
since=since,
until=until,
tenant=tenant,
format_=format_,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    actor: str | Unset = UNSET,
    action: str | Unset = UNSET,
    since: int | Unset = UNSET,
    until: int | Unset = UNSET,
    tenant: str | Unset = UNSET,
    format_: str | Unset = UNSET,

) -> Response[ErrorResponse | str]:
    """ `GET /audit-log/export`: every event the listing's filters match,
    newest first, streamed page by page as CSV or JSON Lines (D8, finding
    36). Who may see what is the listing's.

    Args:
        actor (str | Unset):
        action (str | Unset):
        since (int | Unset):
        until (int | Unset):
        tenant (str | Unset):
        format_ (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | str]
     """


    kwargs = _get_kwargs(
        actor=actor,
action=action,
since=since,
until=until,
tenant=tenant,
format_=format_,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    actor: str | Unset = UNSET,
    action: str | Unset = UNSET,
    since: int | Unset = UNSET,
    until: int | Unset = UNSET,
    tenant: str | Unset = UNSET,
    format_: str | Unset = UNSET,

) -> ErrorResponse | str | None:
    """ `GET /audit-log/export`: every event the listing's filters match,
    newest first, streamed page by page as CSV or JSON Lines (D8, finding
    36). Who may see what is the listing's.

    Args:
        actor (str | Unset):
        action (str | Unset):
        since (int | Unset):
        until (int | Unset):
        tenant (str | Unset):
        format_ (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | str
     """


    return (await asyncio_detailed(
        client=client,
actor=actor,
action=action,
since=since,
until=until,
tenant=tenant,
format_=format_,

    )).parsed
