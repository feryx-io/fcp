from http import HTTPStatus
from typing import Any, cast
from urllib.parse import quote

import httpx

from ...client import AuthenticatedClient, Client
from ...types import Response, UNSET
from ... import errors

from ...models.error_response import ErrorResponse
from ...types import UNSET, Unset
from typing import cast



def _get_kwargs(
    *,
    to: str,
    from_: str | Unset = UNSET,
    source_ip: str | Unset = UNSET,

) -> dict[str, Any]:
    

    

    params: dict[str, Any] = {}

    params["to"] = to

    params["from"] = from_

    params["source_ip"] = source_ip


    params = {k: v for k, v in params.items() if v is not UNSET and v is not None}


    _kwargs: dict[str, Any] = {
        "method": "get",
        "url": "/api/v1/routing/preview",
        "params": params,
    }


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
    *,
    client: AuthenticatedClient | Client,
    to: str,
    from_: str | Unset = UNSET,
    source_ip: str | Unset = UNSET,

) -> Response[Any | ErrorResponse]:
    """ `GET /v1/routing/preview?to=…[&from=…][&source_ip=…]`: what a live call
    manager would do with this tenant's call — the policy and rule that
    decide it and the trunks it would try, in order, each with its price
    (`docs/TENANT_TRUNKS.md` §12, phase 6d).

    Args:
        to (str):
        from_ (str | Unset):
        source_ip (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        to=to,
from_=from_,
source_ip=source_ip,

    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)

def sync(
    *,
    client: AuthenticatedClient | Client,
    to: str,
    from_: str | Unset = UNSET,
    source_ip: str | Unset = UNSET,

) -> Any | ErrorResponse | None:
    """ `GET /v1/routing/preview?to=…[&from=…][&source_ip=…]`: what a live call
    manager would do with this tenant's call — the policy and rule that
    decide it and the trunks it would try, in order, each with its price
    (`docs/TENANT_TRUNKS.md` §12, phase 6d).

    Args:
        to (str):
        from_ (str | Unset):
        source_ip (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return sync_detailed(
        client=client,
to=to,
from_=from_,
source_ip=source_ip,

    ).parsed

async def asyncio_detailed(
    *,
    client: AuthenticatedClient | Client,
    to: str,
    from_: str | Unset = UNSET,
    source_ip: str | Unset = UNSET,

) -> Response[Any | ErrorResponse]:
    """ `GET /v1/routing/preview?to=…[&from=…][&source_ip=…]`: what a live call
    manager would do with this tenant's call — the policy and rule that
    decide it and the trunks it would try, in order, each with its price
    (`docs/TENANT_TRUNKS.md` §12, phase 6d).

    Args:
        to (str):
        from_ (str | Unset):
        source_ip (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[Any | ErrorResponse]
     """


    kwargs = _get_kwargs(
        to=to,
from_=from_,
source_ip=source_ip,

    )

    response = await client.get_async_httpx_client().request(
        **kwargs
    )

    return _build_response(client=client, response=response)

async def asyncio(
    *,
    client: AuthenticatedClient | Client,
    to: str,
    from_: str | Unset = UNSET,
    source_ip: str | Unset = UNSET,

) -> Any | ErrorResponse | None:
    """ `GET /v1/routing/preview?to=…[&from=…][&source_ip=…]`: what a live call
    manager would do with this tenant's call — the policy and rule that
    decide it and the trunks it would try, in order, each with its price
    (`docs/TENANT_TRUNKS.md` §12, phase 6d).

    Args:
        to (str):
        from_ (str | Unset):
        source_ip (str | Unset):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Any | ErrorResponse
     """


    return (await asyncio_detailed(
        client=client,
to=to,
from_=from_,
source_ip=source_ip,

    )).parsed
