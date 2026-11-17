from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...models.upvote_question_request import UpvoteQuestionRequest
from typing import cast



def _get_kwargs(
    room: str,
    qid: str,
    *,
    body: UpvoteQuestionRequest,

) -> dict[str, Any]:
    headers: dict[str, Any] = {}


    

    

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/v1/me/conferences/live/{room}/questions/{qid}/upvote".format(room=quote(str(room), safe=""),qid=quote(str(qid), safe=""),),
    }

    _kwargs["json"] = body.to_dict()

    headers["Content-Type"] = "application/json"

    _kwargs["headers"] = headers
    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Any | ErrorResponse:
    if response.status_code == 200:
        response_200 = response.json()
        return response_200

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
    room: str,
    qid: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpvoteQuestionRequest,

) -> Response[Any | ErrorResponse]:
    """ POST /me/conferences/live/:room/questions/:qid/upvote — upvote another's
    question from an app in the room, or take the upvote back.

    Args:
        room (str):
        qid (str):
        body (UpvoteQuestionRequest): The body of `POST
            /me/conferences/live/:room/questions/:qid/upvote`.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        room=room,
qid=qid,
body=body,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    room: str,
    qid: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpvoteQuestionRequest,

) -> Any | ErrorResponse | None:
    """ POST /me/conferences/live/:room/questions/:qid/upvote — upvote another's
    question from an app in the room, or take the upvote back.

    Args:
        room (str):
        qid (str):
        body (UpvoteQuestionRequest): The body of `POST
            /me/conferences/live/:room/questions/:qid/upvote`.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return sync_detailed(
        room=room,
qid=qid,
client=client,
body=body,

    ).parsed

async def asyncio_detailed(
    room: str,
    qid: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpvoteQuestionRequest,

) -> Response[Any | ErrorResponse]:
    """ POST /me/conferences/live/:room/questions/:qid/upvote — upvote another's
    question from an app in the room, or take the upvote back.

    Args:
        room (str):
        qid (str):
        body (UpvoteQuestionRequest): The body of `POST
            /me/conferences/live/:room/questions/:qid/upvote`.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        room=room,
qid=qid,
body=body,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    room: str,
    qid: str,
    *,
    client: AuthenticatedClient | Client,
    body: UpvoteQuestionRequest,

) -> Any | ErrorResponse | None:
    """ POST /me/conferences/live/:room/questions/:qid/upvote — upvote another's
    question from an app in the room, or take the upvote back.

    Args:
        room (str):
        qid (str):
        body (UpvoteQuestionRequest): The body of `POST
            /me/conferences/live/:room/questions/:qid/upvote`.

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return (await asyncio_detailed(
        room=room,
qid=qid,
client=client,
body=body,

    )).parsed
