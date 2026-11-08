from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.class_of_service_request import ClassOfServiceRequest
from ...models.class_of_service_response import ClassOfServiceResponse
from ...models.error_response import ErrorResponse
from typing import cast



def _get_kwargs(
    id: str,
    *,
    body: ClassOfServiceRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "put",
        "url": "/api/v1/classes-of-service/{id}".format(id=quote(str(id), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ClassOfServiceResponse | ErrorResponse:
    if response.status_code == 200:
        response_200 = ClassOfServiceResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ClassOfServiceResponse | ErrorResponse]:
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
    body: ClassOfServiceRequest,

) -> Response[ClassOfServiceResponse | ErrorResponse]:
    """ PUT /api/v1/classes-of-service/:id — the whole class, replaced.

    Args:
        id (str):
        body (ClassOfServiceRequest): What a class is written as. `allowed` unset allows
            everything;
            `max_calls` unset is two, `null` no limit; `video` unset allows it.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ClassOfServiceResponse | ErrorResponse]
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
    body: ClassOfServiceRequest,

) -> ClassOfServiceResponse | ErrorResponse | None:
    """ PUT /api/v1/classes-of-service/:id — the whole class, replaced.

    Args:
        id (str):
        body (ClassOfServiceRequest): What a class is written as. `allowed` unset allows
            everything;
            `max_calls` unset is two, `null` no limit; `video` unset allows it.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ClassOfServiceResponse | ErrorResponse
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
    body: ClassOfServiceRequest,

) -> Response[ClassOfServiceResponse | ErrorResponse]:
    """ PUT /api/v1/classes-of-service/:id — the whole class, replaced.

    Args:
        id (str):
        body (ClassOfServiceRequest): What a class is written as. `allowed` unset allows
            everything;
            `max_calls` unset is two, `null` no limit; `video` unset allows it.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ClassOfServiceResponse | ErrorResponse]
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
    body: ClassOfServiceRequest,

) -> ClassOfServiceResponse | ErrorResponse | None:
    """ PUT /api/v1/classes-of-service/:id — the whole class, replaced.

    Args:
        id (str):
        body (ClassOfServiceRequest): What a class is written as. `allowed` unset allows
            everything;
            `max_calls` unset is two, `null` no limit; `video` unset allows it.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ClassOfServiceResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        id=id,
client=client,
body=body,

    )).parsed
