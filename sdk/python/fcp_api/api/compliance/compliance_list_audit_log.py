from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.page_audit_event_response import PageAuditEventResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    actor: str | Unset = UNSET,
    action: str | Unset = UNSET,
    since: int | Unset = UNSET,
    until: int | Unset = UNSET,
    api_request_id: str | Unset = UNSET,
    tenant: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["actor"] = actor

    params["action"] = action

    params["since"] = since

    params["until"] = until

    params["api_request_id"] = api_request_id

    params["tenant"] = tenant

    params["limit"] = limit

    params["cursor"] = cursor


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/audit-log",
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | PageAuditEventResponse:
    if response.status_code == 200:
        response_200 = PageAuditEventResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | PageAuditEventResponse]:
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
    api_request_id: str | Unset = UNSET,
    tenant: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> Response[ErrorResponse | PageAuditEventResponse]:
    """ A row belongs to the tenant acted on (docs/TENANT_SCOPE.md §3.3).
    Tenant admins see their tenant's rows, platform administrators' actions
    on it included; a platform administrator sees every row, one tenant's
    (`tenant` or the picked tenant) or the platform's own.

    Args:
        actor (str | Unset):
        action (str | Unset):
        since (int | Unset):
        until (int | Unset):
        api_request_id (str | Unset):
        tenant (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageAuditEventResponse]
     """


    kwargs = _get_kwargs(
        actor=actor,
action=action,
since=since,
until=until,
api_request_id=api_request_id,
tenant=tenant,
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
    actor: str | Unset = UNSET,
    action: str | Unset = UNSET,
    since: int | Unset = UNSET,
    until: int | Unset = UNSET,
    api_request_id: str | Unset = UNSET,
    tenant: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> ErrorResponse | PageAuditEventResponse | None:
    """ A row belongs to the tenant acted on (docs/TENANT_SCOPE.md §3.3).
    Tenant admins see their tenant's rows, platform administrators' actions
    on it included; a platform administrator sees every row, one tenant's
    (`tenant` or the picked tenant) or the platform's own.

    Args:
        actor (str | Unset):
        action (str | Unset):
        since (int | Unset):
        until (int | Unset):
        api_request_id (str | Unset):
        tenant (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageAuditEventResponse
     """


    return sync_detailed(
        client=client,
actor=actor,
action=action,
since=since,
until=until,
api_request_id=api_request_id,
tenant=tenant,
limit=limit,
cursor=cursor,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    actor: str | Unset = UNSET,
    action: str | Unset = UNSET,
    since: int | Unset = UNSET,
    until: int | Unset = UNSET,
    api_request_id: str | Unset = UNSET,
    tenant: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> Response[ErrorResponse | PageAuditEventResponse]:
    """ A row belongs to the tenant acted on (docs/TENANT_SCOPE.md §3.3).
    Tenant admins see their tenant's rows, platform administrators' actions
    on it included; a platform administrator sees every row, one tenant's
    (`tenant` or the picked tenant) or the platform's own.

    Args:
        actor (str | Unset):
        action (str | Unset):
        since (int | Unset):
        until (int | Unset):
        api_request_id (str | Unset):
        tenant (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PageAuditEventResponse]
     """


    kwargs = _get_kwargs(
        actor=actor,
action=action,
since=since,
until=until,
api_request_id=api_request_id,
tenant=tenant,
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
    actor: str | Unset = UNSET,
    action: str | Unset = UNSET,
    since: int | Unset = UNSET,
    until: int | Unset = UNSET,
    api_request_id: str | Unset = UNSET,
    tenant: str | Unset = UNSET,
    limit: int | Unset = UNSET,
    cursor: str | Unset = UNSET,

) -> ErrorResponse | PageAuditEventResponse | None:
    """ A row belongs to the tenant acted on (docs/TENANT_SCOPE.md §3.3).
    Tenant admins see their tenant's rows, platform administrators' actions
    on it included; a platform administrator sees every row, one tenant's
    (`tenant` or the picked tenant) or the platform's own.

    Args:
        actor (str | Unset):
        action (str | Unset):
        since (int | Unset):
        until (int | Unset):
        api_request_id (str | Unset):
        tenant (str | Unset):
        limit (int | Unset):
        cursor (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PageAuditEventResponse
     """


    return (await asyncio_detailed(
        client=client,
actor=actor,
action=action,
since=since,
until=until,
api_request_id=api_request_id,
tenant=tenant,
limit=limit,
cursor=cursor,

    )).parsed
