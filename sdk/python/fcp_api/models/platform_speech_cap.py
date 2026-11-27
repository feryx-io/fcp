from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..models.cap_unit import CapUnit






T = TypeVar("T", bound="PlatformSpeechCap")



@_attrs_define
class PlatformSpeechCap:
    """ A platform provider's default cap for every tenant.

        Attributes:
            provider (str):
            unit (CapUnit): What a cap counts (`docs/TENANT_RECOGNITION.md` R3): characters a
                provider synthesized, or minutes a recognizer heard (counted in
                milliseconds).
            monthly (int): How many of `unit` a month.
     """

    provider: str
    unit: CapUnit
    monthly: int
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        provider = self.provider

        unit = self.unit.value

        monthly = self.monthly


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "provider": provider,
            "unit": unit,
            "monthly": monthly,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        provider = d.pop("provider")

        unit = CapUnit(d.pop("unit"))




        monthly = d.pop("monthly")

        platform_speech_cap = cls(
            provider=provider,
            unit=unit,
            monthly=monthly,
        )


        platform_speech_cap.additional_properties = d
        return platform_speech_cap

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
