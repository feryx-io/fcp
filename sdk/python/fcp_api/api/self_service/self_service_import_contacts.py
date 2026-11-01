from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.contact_import_response import ContactImportResponse
from ...models.error_response import ErrorResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    body: list[int],
    format_: str | Unset = UNSET,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    params: dict[str, Any] = {}

    params["format"] = format_


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/me/contacts/import",
        "params": params,
    }

    _kwargs["content"] = body.payload
    headers["Content-Type"] = "application/octet-stream"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ContactImportResponse | ErrorResponse:
    if response.status_code == 200:
        response_200 = ContactImportResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ContactImportResponse | ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: list[int],
    format_: str | Unset = UNSET,

) -> Response[ContactImportResponse | ErrorResponse]:
    """ POST /me/contacts/import — contacts from a vCard or CSV file (the body),
    kept on the user's account. One already kept, with the same name and
    numbers, is not made again. The user's apps hear one
    `user.contact_changed` (`change`: `imported`) for the lot.

    Args:
        format_ (str | Unset):
        body (list[int]):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ContactImportResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        body=body,
format_=format_,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    body: list[int],
    format_: str | Unset = UNSET,

) -> ContactImportResponse | ErrorResponse | None:
    """ POST /me/contacts/import — contacts from a vCard or CSV file (the body),
    kept on the user's account. One already kept, with the same name and
    numbers, is not made again. The user's apps hear one
    `user.contact_changed` (`change`: `imported`) for the lot.

    Args:
        format_ (str | Unset):
        body (list[int]):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ContactImportResponse | ErrorResponse
     """


    return sync_detailed(
        client=client,
body=body,
format_=format_,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    body: list[int],
    format_: str | Unset = UNSET,

) -> Response[ContactImportResponse | ErrorResponse]:
    """ POST /me/contacts/import — contacts from a vCard or CSV file (the body),
    kept on the user's account. One already kept, with the same name and
    numbers, is not made again. The user's apps hear one
    `user.contact_changed` (`change`: `imported`) for the lot.

    Args:
        format_ (str | Unset):
        body (list[int]):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ContactImportResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        body=body,
format_=format_,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    body: list[int],
    format_: str | Unset = UNSET,

) -> ContactImportResponse | ErrorResponse | None:
    """ POST /me/contacts/import — contacts from a vCard or CSV file (the body),
    kept on the user's account. One already kept, with the same name and
    numbers, is not made again. The user's apps hear one
    `user.contact_changed` (`change`: `imported`) for the lot.

    Args:
        format_ (str | Unset):
        body (list[int]):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ContactImportResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        client=client,
body=body,
format_=format_,

    )).parsed
