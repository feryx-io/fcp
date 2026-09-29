from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.me_set_greeting_active_request import MeSetGreetingActiveRequest
from typing import cast



def _get_kwargs(
    greeting_type: str,
    *,
    body: MeSetGreetingActiveRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "put",
        "url": "/api/v1/me/voicemail/greetings/{greeting_type}/active".format(greeting_type=quote(str(greeting_type), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Any | ErrorResponse:
    if response.status_code == 200:
        response_200 = response.json()
        return response_200

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
    greeting_type: str,
    *,
    client: AuthenticatedClient | Client,
    body: MeSetGreetingActiveRequest,

) -> Response[Any | ErrorResponse]:
    """ PUT /me/voicemail/greetings/:type/active

    Args:
        greeting_type (str):
        body (MeSetGreetingActiveRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        greeting_type=greeting_type,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    greeting_type: str,
    *,
    client: AuthenticatedClient | Client,
    body: MeSetGreetingActiveRequest,

) -> Any | ErrorResponse | None:
    """ PUT /me/voicemail/greetings/:type/active

    Args:
        greeting_type (str):
        body (MeSetGreetingActiveRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return sync_detailed(
        greeting_type=greeting_type,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    greeting_type: str,
    *,
    client: AuthenticatedClient | Client,
    body: MeSetGreetingActiveRequest,

) -> Response[Any | ErrorResponse]:
    """ PUT /me/voicemail/greetings/:type/active

    Args:
        greeting_type (str):
        body (MeSetGreetingActiveRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        greeting_type=greeting_type,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    greeting_type: str,
    *,
    client: AuthenticatedClient | Client,
    body: MeSetGreetingActiveRequest,

) -> Any | ErrorResponse | None:
    """ PUT /me/voicemail/greetings/:type/active

    Args:
        greeting_type (str):
        body (MeSetGreetingActiveRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return (await asyncio_detailed(
        greeting_type=greeting_type,
client=client,
body=body,

    )).parsed
