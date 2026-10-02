from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.saml_post import SamlPost
from typing import cast



def _get_kwargs(
    id: str,
    *,
    body: SamlPost,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/auth/saml/{id}/acs".format(id=quote(str(id), safe=""),),
    }

    _kwargs["data"] = body.to_dict()
    headers["Content-Type"] = "application/x-www-form-urlencoded"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Any | ErrorResponse:
    if response.status_code == 302:
        response_302 = cast(Any, None)
        return response_302

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
    id: str,
    *,
    client: AuthenticatedClient | Client,
    body: SamlPost,

) -> Response[Any | ErrorResponse]:
    """ POST /api/v1/auth/saml/{id}/acs — a SAML provider's answer: to the
    login page with a one-time code, or with the reason it failed.

    Args:
        id (str):
        body (SamlPost): What a SAML provider posts back (the HTTP-POST binding).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
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
    body: SamlPost,

) -> Any | ErrorResponse | None:
    """ POST /api/v1/auth/saml/{id}/acs — a SAML provider's answer: to the
    login page with a one-time code, or with the reason it failed.

    Args:
        id (str):
        body (SamlPost): What a SAML provider posts back (the HTTP-POST binding).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
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
    body: SamlPost,

) -> Response[Any | ErrorResponse]:
    """ POST /api/v1/auth/saml/{id}/acs — a SAML provider's answer: to the
    login page with a one-time code, or with the reason it failed.

    Args:
        id (str):
        body (SamlPost): What a SAML provider posts back (the HTTP-POST binding).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
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
    body: SamlPost,

) -> Any | ErrorResponse | None:
    """ POST /api/v1/auth/saml/{id}/acs — a SAML provider's answer: to the
    login page with a one-time code, or with the reason it failed.

    Args:
        id (str):
        body (SamlPost): What a SAML provider posts back (the HTTP-POST binding).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return (await asyncio_detailed(
        id=id,
client=client,
body=body,

    )).parsed
