from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="PageSpeechUsagePeriodDataItem")



@_attrs_define
class PageSpeechUsagePeriodDataItem:
    """ What the tenant's speech cost in one period with one provider.

        Attributes:
            period (str): `YYYY-MM` or `YYYY-MM-DD` (UTC).
            provider (str): A platform provider's name, or `account:<uuid>`.
            label (str): What to show: the provider, or the account's name.
            requests (int):
            characters (int):
            synthesized_characters (int): What the provider spoke, and bills for; the rest came from the cache.
            estimated_cost (float | None | Unset): The synthesized characters at the provider's price per million
                (the platform's, or the account's own), where one is set.
     """

    period: str
    provider: str
    label: str
    requests: int
    characters: int
    synthesized_characters: int
    estimated_cost: float | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        period = self.period

        provider = self.provider

        label = self.label

        requests = self.requests

        characters = self.characters

        synthesized_characters = self.synthesized_characters

        estimated_cost: float | None | Unset
        if isinstance(self.estimated_cost, Unset):
            estimated_cost = UNSET
        else:
            estimated_cost = self.estimated_cost


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
        if estimated_cost is not UNSET:
            field_dict["estimated_cost"] = estimated_cost

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

        def _parse_estimated_cost(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        estimated_cost = _parse_estimated_cost(d.pop("estimated_cost", UNSET))


        page_speech_usage_period_data_item = cls(
            period=period,
            provider=provider,
            label=label,
            requests=requests,
            characters=characters,
            synthesized_characters=synthesized_characters,
            estimated_cost=estimated_cost,
        )


        page_speech_usage_period_data_item.additional_properties = d
        return page_speech_usage_period_data_item

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
