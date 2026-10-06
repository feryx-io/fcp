from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    client_id: str,
    redirect_uri: str,
    code_challenge: str,
    code_challenge_method: str,
    state: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["client_id"] = client_id

    params["redirect_uri"] = redirect_uri

    params["code_challenge"] = code_challenge

    params["code_challenge_method"] = code_challenge_method

    params["state"] = state


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/auth/app/authorize",
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Any | ErrorResponse:
    if response.status_code == 303:
        response_303 = cast(Any, None)
        return response_303

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
    *,
    client: AuthenticatedClient | Client,
    client_id: str,
    redirect_uri: str,
    code_challenge: str,
    code_challenge_method: str,
    state: str | Unset = UNSET,

) -> Response[Any | ErrorResponse]:
    """ GET /api/v1/auth/app/authorize — an app begins its sign-in in the
    browser, which is sent to the admin UI's sign-in to finish it.

    Args:
        client_id (str):
        redirect_uri (str):
        code_challenge (str):
        code_challenge_method (str):
        state (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        client_id=client_id,
redirect_uri=redirect_uri,
code_challenge=code_challenge,
code_challenge_method=code_challenge_method,
state=state,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    client_id: str,
    redirect_uri: str,
    code_challenge: str,
    code_challenge_method: str,
    state: str | Unset = UNSET,

) -> Any | ErrorResponse | None:
    """ GET /api/v1/auth/app/authorize — an app begins its sign-in in the
    browser, which is sent to the admin UI's sign-in to finish it.

    Args:
        client_id (str):
        redirect_uri (str):
        code_challenge (str):
        code_challenge_method (str):
        state (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return sync_detailed(
        client=client,
client_id=client_id,
redirect_uri=redirect_uri,
code_challenge=code_challenge,
code_challenge_method=code_challenge_method,
state=state,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    client_id: str,
    redirect_uri: str,
    code_challenge: str,
    code_challenge_method: str,
    state: str | Unset = UNSET,

) -> Response[Any | ErrorResponse]:
    """ GET /api/v1/auth/app/authorize — an app begins its sign-in in the
    browser, which is sent to the admin UI's sign-in to finish it.

    Args:
        client_id (str):
        redirect_uri (str):
        code_challenge (str):
        code_challenge_method (str):
        state (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        client_id=client_id,
redirect_uri=redirect_uri,
code_challenge=code_challenge,
code_challenge_method=code_challenge_method,
state=state,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    client_id: str,
    redirect_uri: str,
    code_challenge: str,
    code_challenge_method: str,
    state: str | Unset = UNSET,

) -> Any | ErrorResponse | None:
    """ GET /api/v1/auth/app/authorize — an app begins its sign-in in the
    browser, which is sent to the admin UI's sign-in to finish it.

    Args:
        client_id (str):
        redirect_uri (str):
        code_challenge (str):
        code_challenge_method (str):
        state (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return (await asyncio_detailed(
        client=client,
client_id=client_id,
redirect_uri=redirect_uri,
code_challenge=code_challenge,
code_challenge_method=code_challenge_method,
state=state,

    )).parsed
