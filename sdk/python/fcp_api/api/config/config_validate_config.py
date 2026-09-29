from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.update_config_request import UpdateConfigRequest
from ...models.validate_config_response import ValidateConfigResponse
from typing import cast



def _get_kwargs(
    service: str,
    section: str,
    *,
    body: UpdateConfigRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/config/{service}/{section}/validate".format(service=quote(str(service), safe=""),section=quote(str(section), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | ValidateConfigResponse:
    if response.status_code == 200:
        response_200 = ValidateConfigResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | ValidateConfigResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    service: str,
    section: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdateConfigRequest,

) -> Response[ErrorResponse | ValidateConfigResponse]:
    """ POST /api/v1/config/:service/:section/validate -- validate without saving

    Args:
        service (str):
        section (str):
        body (UpdateConfigRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | ValidateConfigResponse]
     """


    kwargs = _get_kwargs(
        service=service,
section=section,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    service: str,
    section: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdateConfigRequest,

) -> ErrorResponse | ValidateConfigResponse | None:
    """ POST /api/v1/config/:service/:section/validate -- validate without saving

    Args:
        service (str):
        section (str):
        body (UpdateConfigRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | ValidateConfigResponse
     """


    return sync_detailed(
        service=service,
section=section,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    service: str,
    section: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdateConfigRequest,

) -> Response[ErrorResponse | ValidateConfigResponse]:
    """ POST /api/v1/config/:service/:section/validate -- validate without saving

    Args:
        service (str):
        section (str):
        body (UpdateConfigRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | ValidateConfigResponse]
     """


    kwargs = _get_kwargs(
        service=service,
section=section,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    service: str,
    section: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpdateConfigRequest,

) -> ErrorResponse | ValidateConfigResponse | None:
    """ POST /api/v1/config/:service/:section/validate -- validate without saving

    Args:
        service (str):
        section (str):
        body (UpdateConfigRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | ValidateConfigResponse
     """


    return (await asyncio_detailed(
        service=service,
section=section,
client=client,
body=body,

    )).parsed
