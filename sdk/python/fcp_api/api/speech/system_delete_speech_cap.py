from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.cap_unit import CapUnit
from ...models.error_response import ErrorResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    provider: str,
    *,
    unit: CapUnit | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    json_unit: str | Unset = UNSET
    if not isinstance(unit, Unset):
        json_unit = unit.value

    params["unit"] = json_unit


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "delete",
        "url": "/api/v1/system/speech/caps/{provider}".format(provider=quote(str(provider), safe=""),),
        "params": params,
    }


    return _kwargs



def _parse_response(*, client: AuthenticatedClient | Client, response: httpx.Response) -> Any | ErrorResponse:
    if response.status_code == 204:
        response_204 = cast(Any, None)
        return response_204

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
    provider: str,
    *,
    client: AuthenticatedClient | Client,
    unit: CapUnit | Unset = UNSET,

) -> Response[Any | ErrorResponse]:
    """ DELETE /system/speech/caps/{provider}

    Args:
        provider (str):
        unit (CapUnit | Unset): What a cap counts (`docs/TENANT_RECOGNITION.md` R3): characters a
            provider synthesized, or minutes a recognizer heard (counted in
            milliseconds).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        provider=provider,
unit=unit,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    provider: str,
    *,
    client: AuthenticatedClient | Client,
    unit: CapUnit | Unset = UNSET,

) -> Any | ErrorResponse | None:
    """ DELETE /system/speech/caps/{provider}

    Args:
        provider (str):
        unit (CapUnit | Unset): What a cap counts (`docs/TENANT_RECOGNITION.md` R3): characters a
            provider synthesized, or minutes a recognizer heard (counted in
            milliseconds).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return sync_detailed(
        provider=provider,
client=client,
unit=unit,

    ).parsed

async def asyncio_detailed(
    provider: str,
    *,
    client: AuthenticatedClient | Client,
    unit: CapUnit | Unset = UNSET,

) -> Response[Any | ErrorResponse]:
    """ DELETE /system/speech/caps/{provider}

    Args:
        provider (str):
        unit (CapUnit | Unset): What a cap counts (`docs/TENANT_RECOGNITION.md` R3): characters a
            provider synthesized, or minutes a recognizer heard (counted in
            milliseconds).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        provider=provider,
unit=unit,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    provider: str,
    *,
    client: AuthenticatedClient | Client,
    unit: CapUnit | Unset = UNSET,

) -> Any | ErrorResponse | None:
    """ DELETE /system/speech/caps/{provider}

    Args:
        provider (str):
        unit (CapUnit | Unset): What a cap counts (`docs/TENANT_RECOGNITION.md` R3): characters a
            provider synthesized, or minutes a recognizer heard (counted in
            milliseconds).

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return (await asyncio_detailed(
        provider=provider,
client=client,
unit=unit,

    )).parsed
