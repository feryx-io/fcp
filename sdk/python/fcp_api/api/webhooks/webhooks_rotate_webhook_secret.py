from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.rotate_webhook_secret_request import RotateWebhookSecretRequest
from ...models.webhook_with_secret import WebhookWithSecret
from typing import cast



def _get_kwargs(
    id: str,
    *,
    body: RotateWebhookSecretRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/webhooks/{id}/rotate-secret".format(id=quote(str(id), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | WebhookWithSecret:
    if response.status_code == 200:
        response_200 = WebhookWithSecret.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | WebhookWithSecret]:
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
    body: RotateWebhookSecretRequest,

) -> Response[ErrorResponse | WebhookWithSecret]:
    """ POST /api/v1/webhooks/:id/rotate-secret

    Args:
        id (str):
        body (RotateWebhookSecretRequest): A rotation.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | WebhookWithSecret]
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
    body: RotateWebhookSecretRequest,

) -> ErrorResponse | WebhookWithSecret | None:
    """ POST /api/v1/webhooks/:id/rotate-secret

    Args:
        id (str):
        body (RotateWebhookSecretRequest): A rotation.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | WebhookWithSecret
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
    body: RotateWebhookSecretRequest,

) -> Response[ErrorResponse | WebhookWithSecret]:
    """ POST /api/v1/webhooks/:id/rotate-secret

    Args:
        id (str):
        body (RotateWebhookSecretRequest): A rotation.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | WebhookWithSecret]
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
    body: RotateWebhookSecretRequest,

) -> ErrorResponse | WebhookWithSecret | None:
    """ POST /api/v1/webhooks/:id/rotate-secret

    Args:
        id (str):
        body (RotateWebhookSecretRequest): A rotation.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | WebhookWithSecret
     """


    return (await asyncio_detailed(
        id=id,
client=client,
body=body,

    )).parsed
