from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.speech_account_kind import SpeechAccountKind
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="SpeechAccountRequest")



@_attrs_define
class SpeechAccountRequest:
    """ An account to make or change. On a change, an unset `secret` keeps the
    one kept.

        Attributes:
            name (str):
            kind (SpeechAccountKind): The vendors a tenant may bring an account with.
            region (None | str | Unset):
            model (None | str | Unset):
            endpoint (None | str | Unset):
            access_key_id (None | str | Unset):
            secret (None | str | Unset): Write-only: the API key, subscription key or AWS secret access key.
     """

    name: str
    kind: SpeechAccountKind
    region: None | str | Unset = UNSET
    model: None | str | Unset = UNSET
    endpoint: None | str | Unset = UNSET
    access_key_id: None | str | Unset = UNSET
    secret: None | str | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        name = self.name

        kind = self.kind.value

        region: None | str | Unset
        if isinstance(self.region, Unset):
            region = UNSET
        else:
            region = self.region

        model: None | str | Unset
        if isinstance(self.model, Unset):
            model = UNSET
        else:
            model = self.model

        endpoint: None | str | Unset
        if isinstance(self.endpoint, Unset):
            endpoint = UNSET
        else:
            endpoint = self.endpoint

        access_key_id: None | str | Unset
        if isinstance(self.access_key_id, Unset):
            access_key_id = UNSET
        else:
            access_key_id = self.access_key_id

        secret: None | str | Unset
        if isinstance(self.secret, Unset):
            secret = UNSET
        else:
            secret = self.secret


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "name": name,
            "kind": kind,
        })
        if region is not UNSET:
            field_dict["region"] = region
        if model is not UNSET:
            field_dict["model"] = model
        if endpoint is not UNSET:
            field_dict["endpoint"] = endpoint
        if access_key_id is not UNSET:
            field_dict["access_key_id"] = access_key_id
        if secret is not UNSET:
            field_dict["secret"] = secret

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        name = d.pop("name")

        kind = SpeechAccountKind(d.pop("kind"))




        def _parse_region(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        region = _parse_region(d.pop("region", UNSET))


        def _parse_model(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        model = _parse_model(d.pop("model", UNSET))


        def _parse_endpoint(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        endpoint = _parse_endpoint(d.pop("endpoint", UNSET))


        def _parse_access_key_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        access_key_id = _parse_access_key_id(d.pop("access_key_id", UNSET))


        def _parse_secret(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        secret = _parse_secret(d.pop("secret", UNSET))


        speech_account_request = cls(
            name=name,
            kind=kind,
            region=region,
            model=model,
            endpoint=endpoint,
            access_key_id=access_key_id,
            secret=secret,
        )

        return speech_account_request

