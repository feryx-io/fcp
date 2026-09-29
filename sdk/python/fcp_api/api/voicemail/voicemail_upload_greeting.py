from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.greeting_response import GreetingResponse
from ...models.greeting_upload import GreetingUpload
from typing import cast



def _get_kwargs(
    id: str,
    *,
    body: GreetingUpload,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/voicemail/mailboxes/{id}/greetings".format(id=quote(str(id), safe=""),),
    }

    _kwargs["files"] = body.to_multipart()

    headers["Content-Type"] = "multipart/form-data; boundary=+++"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | GreetingResponse:
    if response.status_code == 201:
        response_201 = GreetingResponse.from_dict(response.json())



        return response_201

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | GreetingResponse]:
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
    body: GreetingUpload,

) -> Response[ErrorResponse | GreetingResponse]:
    """ Upload a greeting (multipart: greeting_type + audio file).

    Args:
        id (str):
        body (GreetingUpload): `POST /voicemail/mailboxes/:id/greetings`.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | GreetingResponse]
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
    body: GreetingUpload,

) -> ErrorResponse | GreetingResponse | None:
    """ Upload a greeting (multipart: greeting_type + audio file).

    Args:
        id (str):
        body (GreetingUpload): `POST /voicemail/mailboxes/:id/greetings`.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | GreetingResponse
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
    body: GreetingUpload,

) -> Response[ErrorResponse | GreetingResponse]:
    """ Upload a greeting (multipart: greeting_type + audio file).

    Args:
        id (str):
        body (GreetingUpload): `POST /voicemail/mailboxes/:id/greetings`.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | GreetingResponse]
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
    body: GreetingUpload,

) -> ErrorResponse | GreetingResponse | None:
    """ Upload a greeting (multipart: greeting_type + audio file).

    Args:
        id (str):
        body (GreetingUpload): `POST /voicemail/mailboxes/:id/greetings`.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | GreetingResponse
     """


    return (await asyncio_detailed(
        id=id,
client=client,
body=body,

    )).parsed
