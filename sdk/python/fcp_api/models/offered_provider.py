from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="OfferedProvider")



@_attrs_define
class OfferedProvider:
    """ One of the TTS server's providers, as the platform offers it.

        Attributes:
            name (str):
            is_default (bool):
            speaks (bool): Whether it speaks, and whether it hears (`docs/TENANT_RECOGNITION.md`).
            hears (bool):
            is_default_recognizer (bool):
            offered (bool): Whether tenants may choose it.
            price_per_million (float | None | Unset): What the platform charges tenants per million characters, for an
                estimated cost beside their usage (`docs/SPEECH_CAPS.md`).
            price_per_minute (float | None | Unset): Per minute heard, for a recognizer (`docs/TENANT_RECOGNITION.md` R3).
     """

    name: str
    is_default: bool
    speaks: bool
    hears: bool
    is_default_recognizer: bool
    offered: bool
    price_per_million: float | None | Unset = UNSET
    price_per_minute: float | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        name = self.name

        is_default = self.is_default

        speaks = self.speaks

        hears = self.hears

        is_default_recognizer = self.is_default_recognizer

        offered = self.offered

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
            "name": name,
            "is_default": is_default,
            "speaks": speaks,
            "hears": hears,
            "is_default_recognizer": is_default_recognizer,
            "offered": offered,
        })
        if price_per_million is not UNSET:
            field_dict["price_per_million"] = price_per_million
        if price_per_minute is not UNSET:
            field_dict["price_per_minute"] = price_per_minute

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        name = d.pop("name")

        is_default = d.pop("is_default")

        speaks = d.pop("speaks")

        hears = d.pop("hears")

        is_default_recognizer = d.pop("is_default_recognizer")

        offered = d.pop("offered")

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


        offered_provider = cls(
            name=name,
            is_default=is_default,
            speaks=speaks,
            hears=hears,
            is_default_recognizer=is_default_recognizer,
            offered=offered,
            price_per_million=price_per_million,
            price_per_minute=price_per_minute,
        )


        offered_provider.additional_properties = d
        return offered_provider

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
