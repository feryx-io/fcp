from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.meeting_response import MeetingResponse
from ...models.update_meeting_request import UpdateMeetingRequest
from typing import cast



def _get_kwargs(
    id: str,
    *,
    body: UpdateMeetingRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "patch",
        "url": "/api/v1/conferences/meetings/{id}".format(id=quote(str(id), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse | MeetingResponse:
    if response.status_code == 200:
        response_200 = MeetingResponse.from_dict(response.json())



        return response_200

    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse | MeetingResponse]:
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
    body: UpdateMeetingRequest,

) -> Response[ErrorResponse | MeetingResponse]:
    """ PATCH /conferences/meetings/:id — Update a meeting.

    Args:
        id (str):
        body (UpdateMeetingRequest): Update meeting request body (all fields optional; `null`
            clears one
            that may be empty).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | MeetingResponse]
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
    body: UpdateMeetingRequest,

) -> ErrorResponse | MeetingResponse | None:
    """ PATCH /conferences/meetings/:id — Update a meeting.

    Args:
        id (str):
        body (UpdateMeetingRequest): Update meeting request body (all fields optional; `null`
            clears one
            that may be empty).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | MeetingResponse
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
    body: UpdateMeetingRequest,

) -> Response[ErrorResponse | MeetingResponse]:
    """ PATCH /conferences/meetings/:id — Update a meeting.

    Args:
        id (str):
        body (UpdateMeetingRequest): Update meeting request body (all fields optional; `null`
            clears one
            that may be empty).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse | MeetingResponse]
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
    body: UpdateMeetingRequest,

) -> ErrorResponse | MeetingResponse | None:
    """ PATCH /conferences/meetings/:id — Update a meeting.

    Args:
        id (str):
        body (UpdateMeetingRequest): Update meeting request body (all fields optional; `null`
            clears one
            that may be empty).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse | MeetingResponse
     """


    return (await asyncio_detailed(
        id=id,
client=client,
body=body,

    )).parsed
