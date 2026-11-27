from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.speech_account_kind import SpeechAccountKind
from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="SpeechAccountView")



@_attrs_define
class SpeechAccountView:
    """ A tenant's account with a TTS vendor, its key never shown.

        Attributes:
            id (str): `speech-<uuid>`.
            name (str):
            kind (SpeechAccountKind): The vendors a tenant may bring an account with.
            provider (str): What names it as a provider in the settings and requests.
            has_key (bool): Whether a key is kept; the key itself is never returned.
            created_at (str):
            updated_at (str):
            region (None | str | Unset):
            model (None | str | Unset):
            endpoint (None | str | Unset):
            access_key_id (None | str | Unset):
            price_per_million (float | None | Unset): What the vendor charges per million characters, for an estimated
                cost (`docs/SPEECH_CAPS.md`).
            price_per_minute (float | None | Unset): Per minute heard, for a recognizer (`docs/TENANT_RECOGNITION.md` R3).
     """

    id: str
    name: str
    kind: SpeechAccountKind
    provider: str
    has_key: bool
    created_at: str
    updated_at: str
    region: None | str | Unset = UNSET
    model: None | str | Unset = UNSET
    endpoint: None | str | Unset = UNSET
    access_key_id: None | str | Unset = UNSET
    price_per_million: float | None | Unset = UNSET
    price_per_minute: float | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        name = self.name

        kind = self.kind.value

        provider = self.provider

        has_key = self.has_key

        created_at = self.created_at

        updated_at = self.updated_at

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

        price_per_million: float | None | Unset
        if isinstance(self.price_per_million, Unset):
            price_per_million = UNSET
        else:
            price_per_million = self.price_per_million

        price_per_minute: float | None | Unset
        if isinstance(self.price_per_minute, Unset):
            price_per_minute = UNSET
        else:
            price_per_minute = self.price_per_minute


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "name": name,
            "kind": kind,
            "provider": provider,
            "has_key": has_key,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if region is not UNSET:
            field_dict["region"] = region
        if model is not UNSET:
            field_dict["model"] = model
        if endpoint is not UNSET:
            field_dict["endpoint"] = endpoint
        if access_key_id is not UNSET:
            field_dict["access_key_id"] = access_key_id
        if price_per_million is not UNSET:
            field_dict["price_per_million"] = price_per_million
        if price_per_minute is not UNSET:
            field_dict["price_per_minute"] = price_per_minute

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        name = d.pop("name")

        kind = SpeechAccountKind(d.pop("kind"))




        provider = d.pop("provider")

        has_key = d.pop("has_key")

        created_at = d.pop("created_at")

        updated_at = d.pop("updated_at")

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


        def _parse_price_per_million(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        price_per_million = _parse_price_per_million(d.pop("price_per_million", UNSET))


        def _parse_price_per_minute(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        price_per_minute = _parse_price_per_minute(d.pop("price_per_minute", UNSET))


        speech_account_view = cls(
            id=id,
            name=name,
            kind=kind,
            provider=provider,
            has_key=has_key,
            created_at=created_at,
            updated_at=updated_at,
            region=region,
            model=model,
            endpoint=endpoint,
            access_key_id=access_key_id,
            price_per_million=price_per_million,
            price_per_minute=price_per_minute,
        )


        speech_account_view.additional_properties = d
        return speech_account_view

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
