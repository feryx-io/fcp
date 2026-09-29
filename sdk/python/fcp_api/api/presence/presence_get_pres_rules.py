from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from typing import cast



def _get_kwargs(
    aor: str,

) -> dict[str, Any]:
    

    

    

    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/presence/{aor}/pres-rules".format(aor=quote(str(aor), safe=""),),
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> ErrorResponse:
    response_default = ErrorResponse.from_dict(response.json())



    return response_default



def _build_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Response[ErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    aor: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[ErrorResponse]:
    """ GET /presence/:aor/pres-rules — Get authorization policy as XCAP pres-rules XML (RFC 5025).

    Args:
        aor (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse]
     """


    kwargs = _get_kwargs(
        aor=aor,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    aor: str,
    *,
    client: AuthenticatedClient | Client,

) -> ErrorResponse | None:
    """ GET /presence/:aor/pres-rules — Get authorization policy as XCAP pres-rules XML (RFC 5025).

    Args:
        aor (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse
     """


    return sync_detailed(
        aor=aor,
client=client,

    ).parsed

async def asyncio_detailed(
    aor: str,
    *,
    client: AuthenticatedClient | Client,

) -> Response[ErrorResponse]:
    """ GET /presence/:aor/pres-rules — Get authorization policy as XCAP pres-rules XML (RFC 5025).

    Args:
        aor (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ErrorResponse]
     """


    kwargs = _get_kwargs(
        aor=aor,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    aor: str,
    *,
    client: AuthenticatedClient | Client,

) -> ErrorResponse | None:
    """ GET /presence/:aor/pres-rules — Get authorization policy as XCAP pres-rules XML (RFC 5025).

    Args:
        aor (str):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ErrorResponse
     """


    return (await asyncio_detailed(
        aor=aor,
client=client,

    )).parsed
