from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.presence_response import PresenceResponse
from ...models.update_presence_request import UpdatePresenceRequest
from typing import cast



def _get_kwargs(
    aor: str,
    *,
    body: UpdatePresenceRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "put",
        "url": "/api/v1/presence/{aor}".format(aor=quote(str(aor), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | PresenceResponse:
    if response.status_code == 200:
        response_200 = PresenceResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | PresenceResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    aor: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdatePresenceRequest,

) -> Response[ErrorResponse | PresenceResponse]:
    """ PUT /presence/:aor — Set presence state via API.

    Args:
        aor (str):
        body (UpdatePresenceRequest): Update presence request body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PresenceResponse]
     """


    kwargs = _get_kwargs(
        aor=aor,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    aor: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdatePresenceRequest,

) -> ErrorResponse | PresenceResponse | None:
    """ PUT /presence/:aor — Set presence state via API.

    Args:
        aor (str):
        body (UpdatePresenceRequest): Update presence request body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PresenceResponse
     """


    return sync_detailed(
        aor=aor,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    aor: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdatePresenceRequest,

) -> Response[ErrorResponse | PresenceResponse]:
    """ PUT /presence/:aor — Set presence state via API.

    Args:
        aor (str):
        body (UpdatePresenceRequest): Update presence request body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PresenceResponse]
     """


    kwargs = _get_kwargs(
        aor=aor,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    aor: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdatePresenceRequest,

) -> ErrorResponse | PresenceResponse | None:
    """ PUT /presence/:aor — Set presence state via API.

    Args:
        aor (str):
        body (UpdatePresenceRequest): Update presence request body.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PresenceResponse
     """


    return (await asyncio_detailed(
        aor=aor,
client=client,
body=body,

    )).parsed
