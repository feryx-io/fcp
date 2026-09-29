from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.notify_call_request import NotifyCallRequest
from ...models.notify_call_response import NotifyCallResponse
from typing import cast



def _get_kwargs(
    *,
    body: NotifyCallRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/emergency/notify-call",
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | NotifyCallResponse:
    if response.status_code == 200:
        response_200 = NotifyCallResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | NotifyCallResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: NotifyCallRequest,

) -> Response[ErrorResponse | NotifyCallResponse]:
    """ `POST /emergency/notify-call` — ring someone and speak the notice to
    them: the call channel of on-site notification, which the events
    server asks this node for (`docs/E911_BLOCK_2.md` §8.1).

    Args:
        body (NotifyCallRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | NotifyCallResponse]
     """


    kwargs = _get_kwargs(
        body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    body: NotifyCallRequest,

) -> ErrorResponse | NotifyCallResponse | None:
    """ `POST /emergency/notify-call` — ring someone and speak the notice to
    them: the call channel of on-site notification, which the events
    server asks this node for (`docs/E911_BLOCK_2.md` §8.1).

    Args:
        body (NotifyCallRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | NotifyCallResponse
     """


    return sync_detailed(
        client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: NotifyCallRequest,

) -> Response[ErrorResponse | NotifyCallResponse]:
    """ `POST /emergency/notify-call` — ring someone and speak the notice to
    them: the call channel of on-site notification, which the events
    server asks this node for (`docs/E911_BLOCK_2.md` §8.1).

    Args:
        body (NotifyCallRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | NotifyCallResponse]
     """


    kwargs = _get_kwargs(
        body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    body: NotifyCallRequest,

) -> ErrorResponse | NotifyCallResponse | None:
    """ `POST /emergency/notify-call` — ring someone and speak the notice to
    them: the call channel of on-site notification, which the events
    server asks this node for (`docs/E911_BLOCK_2.md` §8.1).

    Args:
        body (NotifyCallRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | NotifyCallResponse
     """


    return (await asyncio_detailed(
        client=client,
body=body,

    )).parsed
