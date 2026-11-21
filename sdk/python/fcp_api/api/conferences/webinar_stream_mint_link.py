from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.stream_link_request import StreamLinkRequest
from ...models.stream_link_response import StreamLinkResponse
from typing import cast



def _get_kwargs(
    id: str,
    *,
    body: StreamLinkRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/conferences/meetings/{id}/stream/links".format(id=quote(str(id), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | StreamLinkResponse:
    if response.status_code == 201:
        response_201 = StreamLinkResponse.from_dict(response.json())



        return response_201

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | StreamLinkResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    id: str,
    *,
    client: AuthenticatedClient | Client,
    body: StreamLinkRequest,

) -> Response[ErrorResponse | StreamLinkResponse]:
    """ POST /conferences/meetings/:id/stream/links — a link to the webinar's
    stream for viewers who are not on FCP.

    Args:
        id (str):
        body (StreamLinkRequest): `POST …/stream/links` body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | StreamLinkResponse]
     """


    kwargs = _get_kwargs(
        id=id,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    id: str,
    *,
    client: AuthenticatedClient | Client,
    body: StreamLinkRequest,

) -> ErrorResponse | StreamLinkResponse | None:
    """ POST /conferences/meetings/:id/stream/links — a link to the webinar's
    stream for viewers who are not on FCP.

    Args:
        id (str):
        body (StreamLinkRequest): `POST …/stream/links` body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | StreamLinkResponse
     """


    return sync_detailed(
        id=id,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    id: str,
    *,
    client: AuthenticatedClient | Client,
    body: StreamLinkRequest,

) -> Response[ErrorResponse | StreamLinkResponse]:
    """ POST /conferences/meetings/:id/stream/links — a link to the webinar's
    stream for viewers who are not on FCP.

    Args:
        id (str):
        body (StreamLinkRequest): `POST …/stream/links` body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | StreamLinkResponse]
     """


    kwargs = _get_kwargs(
        id=id,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    id: str,
    *,
    client: AuthenticatedClient | Client,
    body: StreamLinkRequest,

) -> ErrorResponse | StreamLinkResponse | None:
    """ POST /conferences/meetings/:id/stream/links — a link to the webinar's
    stream for viewers who are not on FCP.

    Args:
        id (str):
        body (StreamLinkRequest): `POST …/stream/links` body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | StreamLinkResponse
     """


    return (await asyncio_detailed(
        id=id,
client=client,
body=body,

    )).parsed
