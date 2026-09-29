from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.prompt_audio_upload import PromptAudioUpload
from ...models.prompt_response import PromptResponse
from typing import cast



def _get_kwargs(
    id: str,
    *,
    body: PromptAudioUpload,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/prompts/{id}/audio".format(id=quote(str(id), safe=""),),
    }

    _kwargs["files"] = body.to_multipart()

    headers["Content-Type"] = "multipart/form-data; boundary=+++"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | PromptResponse:
    if response.status_code == 200:
        response_200 = PromptResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | PromptResponse]:
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
    body: PromptAudioUpload,

) -> Response[ErrorResponse | PromptResponse]:
    """ POST /api/v1/prompts/:id/audio — Upload/replace audio WAV file

    Args:
        id (str):
        body (PromptAudioUpload): `POST /prompts/:id/audio`.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PromptResponse]
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
    body: PromptAudioUpload,

) -> ErrorResponse | PromptResponse | None:
    """ POST /api/v1/prompts/:id/audio — Upload/replace audio WAV file

    Args:
        id (str):
        body (PromptAudioUpload): `POST /prompts/:id/audio`.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PromptResponse
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
    body: PromptAudioUpload,

) -> Response[ErrorResponse | PromptResponse]:
    """ POST /api/v1/prompts/:id/audio — Upload/replace audio WAV file

    Args:
        id (str):
        body (PromptAudioUpload): `POST /prompts/:id/audio`.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | PromptResponse]
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
    body: PromptAudioUpload,

) -> ErrorResponse | PromptResponse | None:
    """ POST /api/v1/prompts/:id/audio — Upload/replace audio WAV file

    Args:
        id (str):
        body (PromptAudioUpload): `POST /prompts/:id/audio`.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | PromptResponse
     """


    return (await asyncio_detailed(
        id=id,
client=client,
body=body,

    )).parsed
