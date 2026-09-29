from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast






T = TypeVar("T", bound="TrunkStats")



@_attrs_define
class TrunkStats:
    """ Aggregated statistics for a trunk computed from CDRs.

        Attributes:
            total_calls_24h (int): Total calls through this trunk (last 24h)
            answered_calls_24h (int): Answered calls (last 24h)
            asr (float): Answer-Seizure Ratio as percentage (0-100)
            acd_secs (float): Average Call Duration in seconds (answered calls only)
            pdd_ms (float): Average Post-Dial Delay in milliseconds
            hourly_volume (list[int]): Call volume per hour for last 24h (24 buckets, oldest first)
     """

    total_calls_24h: int
    answered_calls_24h: int
    asr: float
    acd_secs: float
    pdd_ms: float
    hourly_volume: list[int]
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        total_calls_24h = self.total_calls_24h

        answered_calls_24h = self.answered_calls_24h

        asr = self.asr

        acd_secs = self.acd_secs

        pdd_ms = self.pdd_ms

        hourly_volume = self.hourly_volume




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "total_calls_24h": total_calls_24h,
            "answered_calls_24h": answered_calls_24h,
            "asr": asr,
            "acd_secs": acd_secs,
            "pdd_ms": pdd_ms,
            "hourly_volume": hourly_volume,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        total_calls_24h = d.pop("total_calls_24h")

        answered_calls_24h = d.pop("answered_calls_24h")

        asr = d.pop("asr")

        acd_secs = d.pop("acd_secs")

        pdd_ms = d.pop("pdd_ms")

        hourly_volume = cast(list[int], d.pop("hourly_volume"))


        trunk_stats = cls(
            total_calls_24h=total_calls_24h,
            answered_calls_24h=answered_calls_24h,
            asr=asr,
            acd_secs=acd_secs,
            pdd_ms=pdd_ms,
            hourly_volume=hourly_volume,
        )


        trunk_stats.additional_properties = d
        return trunk_stats

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
