from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.retrieve_parked_request import RetrieveParkedRequest
from typing import cast



def _get_kwargs(
    slot: str,
    *,
    body: RetrieveParkedRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/parking/calls/{slot}/retrieve".format(slot=quote(str(slot), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Any | ErrorResponse:
    if response.status_code == 202:
        response_202 = cast(Any, None)
        return response_202

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[Any | ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    slot: str,
    *,
    client: AuthenticatedClient | Client,
    body: RetrieveParkedRequest,

) -> Response[Any | ErrorResponse]:
    """ POST /api/v1/parking/calls/:slot/retrieve — ring `target` with the
    call in the slot (202); 404 when the slot is empty.

    Args:
        slot (str):
        body (RetrieveParkedRequest): Where a parked call is sent.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        slot=slot,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    slot: str,
    *,
    client: AuthenticatedClient | Client,
    body: RetrieveParkedRequest,

) -> Any | ErrorResponse | None:
    """ POST /api/v1/parking/calls/:slot/retrieve — ring `target` with the
    call in the slot (202); 404 when the slot is empty.

    Args:
        slot (str):
        body (RetrieveParkedRequest): Where a parked call is sent.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return sync_detailed(
        slot=slot,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    slot: str,
    *,
    client: AuthenticatedClient | Client,
    body: RetrieveParkedRequest,

) -> Response[Any | ErrorResponse]:
    """ POST /api/v1/parking/calls/:slot/retrieve — ring `target` with the
    call in the slot (202); 404 when the slot is empty.

    Args:
        slot (str):
        body (RetrieveParkedRequest): Where a parked call is sent.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        slot=slot,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    slot: str,
    *,
    client: AuthenticatedClient | Client,
    body: RetrieveParkedRequest,

) -> Any | ErrorResponse | None:
    """ POST /api/v1/parking/calls/:slot/retrieve — ring `target` with the
    call in the slot (202); 404 when the slot is empty.

    Args:
        slot (str):
        body (RetrieveParkedRequest): Where a parked call is sent.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return (await asyncio_detailed(
        slot=slot,
client=client,
body=body,

    )).parsed
