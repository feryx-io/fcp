from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="SpeechUsagePeriod")



@_attrs_define
class SpeechUsagePeriod:
    """ What the tenant's speech cost in one period with one provider.

        Attributes:
            period (str): `YYYY-MM` or `YYYY-MM-DD` (UTC).
            provider (str): A platform provider's name, or `account:<uuid>`.
            label (str): What to show: the provider, or the account's name.
            requests (int):
            characters (int):
            synthesized_characters (int): What the provider spoke, and bills for; the rest came from the cache.
     """

    period: str
    provider: str
    label: str
    requests: int
    characters: int
    synthesized_characters: int
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        period = self.period

        provider = self.provider

        label = self.label

        requests = self.requests

        characters = self.characters

        synthesized_characters = self.synthesized_characters


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "period": period,
            "provider": provider,
            "label": label,
            "requests": requests,
            "characters": characters,
            "synthesized_characters": synthesized_characters,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        period = d.pop("period")

        provider = d.pop("provider")

        label = d.pop("label")

        requests = d.pop("requests")

        characters = d.pop("characters")

        synthesized_characters = d.pop("synthesized_characters")

        speech_usage_period = cls(
            period=period,
            provider=provider,
            label=label,
            requests=requests,
            characters=characters,
            synthesized_characters=synthesized_characters,
        )


        speech_usage_period.additional_properties = d
        return speech_usage_period

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
