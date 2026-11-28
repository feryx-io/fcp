from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.download_request import DownloadRequest
from ...models.download_view import DownloadView
from ...models.error_response import ErrorResponse
from typing import cast



def _get_kwargs(
    id: str,
    run: str,
    *,
    body: DownloadRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/system/backups/destinations/{id}/snapshots/{run}/download".format(id=quote(str(id), safe=""),run=quote(str(run), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> DownloadView | ErrorResponse:
    if response.status_code == 202:
        response_202 = DownloadView.from_dict(response.json())



        return response_202

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[DownloadView | ErrorResponse]:
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
    body: DownloadRequest,

) -> Response[DownloadView | ErrorResponse]:
    """ POST /system/backups/destinations/{id}/snapshots/{run}/download — the
    point made a `backup.sh`-layout tar by the backup service, for
    `restore.sh` on any host. Audited.

    Args:
        id (str):
        run (str):
        body (DownloadRequest): What a download needs when the snapshot holds no master key and
            this
            system's is another: the key file downloaded from this page, and its
            passphrase.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[DownloadView | ErrorResponse]
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
    body: DownloadRequest,

) -> DownloadView | ErrorResponse | None:
    """ POST /system/backups/destinations/{id}/snapshots/{run}/download — the
    point made a `backup.sh`-layout tar by the backup service, for
    `restore.sh` on any host. Audited.

    Args:
        id (str):
        run (str):
        body (DownloadRequest): What a download needs when the snapshot holds no master key and
            this
            system's is another: the key file downloaded from this page, and its
            passphrase.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        DownloadView | ErrorResponse
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
    body: DownloadRequest,

) -> Response[DownloadView | ErrorResponse]:
    """ POST /system/backups/destinations/{id}/snapshots/{run}/download — the
    point made a `backup.sh`-layout tar by the backup service, for
    `restore.sh` on any host. Audited.

    Args:
        id (str):
        run (str):
        body (DownloadRequest): What a download needs when the snapshot holds no master key and
            this
            system's is another: the key file downloaded from this page, and its
            passphrase.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[DownloadView | ErrorResponse]
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
    body: DownloadRequest,

) -> DownloadView | ErrorResponse | None:
    """ POST /system/backups/destinations/{id}/snapshots/{run}/download — the
    point made a `backup.sh`-layout tar by the backup service, for
    `restore.sh` on any host. Audited.

    Args:
        id (str):
        run (str):
        body (DownloadRequest): What a download needs when the snapshot holds no master key and
            this
            system's is another: the key file downloaded from this page, and its
            passphrase.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        DownloadView | ErrorResponse
     """


    return (await asyncio_detailed(
        id=id,
run=run,
client=client,
body=body,

    )).parsed
