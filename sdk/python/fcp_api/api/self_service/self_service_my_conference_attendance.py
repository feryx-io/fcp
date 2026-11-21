from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.attendance_response import AttendanceResponse
from ...models.error_response import ErrorResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    id: str,
    *,
    sitting: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["sitting"] = sitting


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/me/conferences/{id}/attendance".format(id=quote(str(id), safe=""),),
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> AttendanceResponse | ErrorResponse:
    if response.status_code == 200:
        response_200 = AttendanceResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[AttendanceResponse | ErrorResponse]:
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
    sitting: str | Unset = UNSET,

) -> Response[AttendanceResponse | ErrorResponse]:
    """ GET /me/conferences/:id/attendance -- who came to a sitting of one of
    the user's own meetings, how long, and who watched its stream, for its
    host: the report an administrator reads at
    `/conferences/meetings/{id}/attendance`.

    Args:
        id (str):
        sitting (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[AttendanceResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        id=id,
sitting=sitting,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    id: str,
    *,
    client: AuthenticatedClient | Client,
    sitting: str | Unset = UNSET,

) -> AttendanceResponse | ErrorResponse | None:
    """ GET /me/conferences/:id/attendance -- who came to a sitting of one of
    the user's own meetings, how long, and who watched its stream, for its
    host: the report an administrator reads at
    `/conferences/meetings/{id}/attendance`.

    Args:
        id (str):
        sitting (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        AttendanceResponse | ErrorResponse
     """


    return sync_detailed(
        id=id,
client=client,
sitting=sitting,

    ).parsed

async def asyncio_detailed(
    id: str,
    *,
    client: AuthenticatedClient | Client,
    sitting: str | Unset = UNSET,

) -> Response[AttendanceResponse | ErrorResponse]:
    """ GET /me/conferences/:id/attendance -- who came to a sitting of one of
    the user's own meetings, how long, and who watched its stream, for its
    host: the report an administrator reads at
    `/conferences/meetings/{id}/attendance`.

    Args:
        id (str):
        sitting (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[AttendanceResponse | ErrorResponse]
     """


    kwargs = _get_kwargs(
        id=id,
sitting=sitting,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    id: str,
    *,
    client: AuthenticatedClient | Client,
    sitting: str | Unset = UNSET,

) -> AttendanceResponse | ErrorResponse | None:
    """ GET /me/conferences/:id/attendance -- who came to a sitting of one of
    the user's own meetings, how long, and who watched its stream, for its
    host: the report an administrator reads at
    `/conferences/meetings/{id}/attendance`.

    Args:
        id (str):
        sitting (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        AttendanceResponse | ErrorResponse
     """


    return (await asyncio_detailed(
        id=id,
client=client,
sitting=sitting,

    )).parsed
