from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.system_restore import SystemRestore
from ...models.system_restore_request import SystemRestoreRequest
from typing import cast



def _get_kwargs(
    id: str,
    run: str,
    *,
    body: SystemRestoreRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/system/backups/destinations/{id}/snapshots/{run}/restore-system".format(id=quote(str(id), safe=""),run=quote(str(run), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | SystemRestore:
    if response.status_code == 202:
        response_202 = SystemRestore.from_dict(response.json())



        return response_202

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | SystemRestore]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    id: str,
    run: str,
    *,
    client: AuthenticatedClient | Client,
    body: SystemRestoreRequest,

) -> Response[ErrorResponse | SystemRestore]:
    """ POST /system/backups/destinations/{id}/snapshots/{run}/restore-system —
    the whole system restored from a point, by the backup service: every
    other service drained and stopped, the database restored beside the old
    one and swapped in by rename, the volumes made as the point holds them,
    the services started. The admin itself is stopped part way; follow it at
    `/system/backups/restores/current` when it is back. Audited.

    Args:
        id (str):
        run (str):
        body (SystemRestoreRequest): A full restore asked for: the point's time typed back, and
            the key file
            when the snapshot holds no key and this system's is another.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | SystemRestore]
     """


    kwargs = _get_kwargs(
        id=id,
run=run,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    id: str,
    run: str,
    *,
    client: AuthenticatedClient | Client,
    body: SystemRestoreRequest,

) -> ErrorResponse | SystemRestore | None:
    """ POST /system/backups/destinations/{id}/snapshots/{run}/restore-system —
    the whole system restored from a point, by the backup service: every
    other service drained and stopped, the database restored beside the old
    one and swapped in by rename, the volumes made as the point holds them,
    the services started. The admin itself is stopped part way; follow it at
    `/system/backups/restores/current` when it is back. Audited.

    Args:
        id (str):
        run (str):
        body (SystemRestoreRequest): A full restore asked for: the point's time typed back, and
            the key file
            when the snapshot holds no key and this system's is another.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | SystemRestore
     """


    return sync_detailed(
        id=id,
run=run,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    id: str,
    run: str,
    *,
    client: AuthenticatedClient | Client,
    body: SystemRestoreRequest,

) -> Response[ErrorResponse | SystemRestore]:
    """ POST /system/backups/destinations/{id}/snapshots/{run}/restore-system —
    the whole system restored from a point, by the backup service: every
    other service drained and stopped, the database restored beside the old
    one and swapped in by rename, the volumes made as the point holds them,
    the services started. The admin itself is stopped part way; follow it at
    `/system/backups/restores/current` when it is back. Audited.

    Args:
        id (str):
        run (str):
        body (SystemRestoreRequest): A full restore asked for: the point's time typed back, and
            the key file
            when the snapshot holds no key and this system's is another.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | SystemRestore]
     """


    kwargs = _get_kwargs(
        id=id,
run=run,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    id: str,
    run: str,
    *,
    client: AuthenticatedClient | Client,
    body: SystemRestoreRequest,

) -> ErrorResponse | SystemRestore | None:
    """ POST /system/backups/destinations/{id}/snapshots/{run}/restore-system —
    the whole system restored from a point, by the backup service: every
    other service drained and stopped, the database restored beside the old
    one and swapped in by rename, the volumes made as the point holds them,
    the services started. The admin itself is stopped part way; follow it at
    `/system/backups/restores/current` when it is back. Audited.

    Args:
        id (str):
        run (str):
        body (SystemRestoreRequest): A full restore asked for: the point's time typed back, and
            the key file
            when the snapshot holds no key and this system's is another.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | SystemRestore
     """


    return (await asyncio_detailed(
        id=id,
run=run,
client=client,
body=body,

    )).parsed
