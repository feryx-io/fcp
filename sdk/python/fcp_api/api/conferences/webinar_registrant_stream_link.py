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
    rid: str,
    *,
    body: StreamLinkRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/conferences/meetings/{id}/registrants/{rid}/stream-link".format(id=quote(str(id), safe=""),rid=quote(str(rid), safe=""),),
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
    rid: str,
    *,
    client: AuthenticatedClient | Client,
    body: StreamLinkRequest,

) -> Response[ErrorResponse | StreamLinkResponse]:
    """ POST /conferences/meetings/:id/registrants/:rid/stream-link — a link to
    the webinar's stream and its watch page for one registration (W8): its
    watching counted under it, refused once it is revoked.

    Args:
        id (str):
        rid (str):
        body (StreamLinkRequest): `POST …/stream/links` body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | StreamLinkResponse]
     """


    kwargs = _get_kwargs(
        id=id,
rid=rid,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    id: str,
    rid: str,
    *,
    client: AuthenticatedClient | Client,
    body: StreamLinkRequest,

) -> ErrorResponse | StreamLinkResponse | None:
    """ POST /conferences/meetings/:id/registrants/:rid/stream-link — a link to
    the webinar's stream and its watch page for one registration (W8): its
    watching counted under it, refused once it is revoked.

    Args:
        id (str):
        rid (str):
        body (StreamLinkRequest): `POST …/stream/links` body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | StreamLinkResponse
     """


    return sync_detailed(
        id=id,
rid=rid,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    id: str,
    rid: str,
    *,
    client: AuthenticatedClient | Client,
    body: StreamLinkRequest,

) -> Response[ErrorResponse | StreamLinkResponse]:
    """ POST /conferences/meetings/:id/registrants/:rid/stream-link — a link to
    the webinar's stream and its watch page for one registration (W8): its
    watching counted under it, refused once it is revoked.

    Args:
        id (str):
        rid (str):
        body (StreamLinkRequest): `POST …/stream/links` body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | StreamLinkResponse]
     """


    kwargs = _get_kwargs(
        id=id,
rid=rid,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    id: str,
    rid: str,
    *,
    client: AuthenticatedClient | Client,
    body: StreamLinkRequest,

) -> ErrorResponse | StreamLinkResponse | None:
    """ POST /conferences/meetings/:id/registrants/:rid/stream-link — a link to
    the webinar's stream and its watch page for one registration (W8): its
    watching counted under it, refused once it is revoked.

    Args:
        id (str):
        rid (str):
        body (StreamLinkRequest): `POST …/stream/links` body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | StreamLinkResponse
     """


    return (await asyncio_detailed(
        id=id,
rid=rid,
client=client,
body=body,

    )).parsed
