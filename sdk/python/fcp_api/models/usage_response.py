from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="UsageResponse")



@_attrs_define
class UsageResponse:
    """ Usage aggregation response.

        Attributes:
            period_start (int):
            period_end (int):
            total_calls (int):
            answered_calls (int):
            total_duration_ms (int):
            inbound_calls (int):
            outbound_calls (int):
            total_cost (float): Sum of rated call costs in the period, in the tenant's rate-plan
                currency (0 when billing is not configured).
     """

    period_start: int
    period_end: int
    total_calls: int
    answered_calls: int
    total_duration_ms: int
    inbound_calls: int
    outbound_calls: int
    total_cost: float
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        period_start = self.period_start

        period_end = self.period_end

        total_calls = self.total_calls

        answered_calls = self.answered_calls

        total_duration_ms = self.total_duration_ms

        inbound_calls = self.inbound_calls

        outbound_calls = self.outbound_calls

        total_cost = self.total_cost


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "period_start": period_start,
            "period_end": period_end,
            "total_calls": total_calls,
            "answered_calls": answered_calls,
            "total_duration_ms": total_duration_ms,
            "inbound_calls": inbound_calls,
            "outbound_calls": outbound_calls,
            "total_cost": total_cost,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        period_start = d.pop("period_start")

        period_end = d.pop("period_end")

        total_calls = d.pop("total_calls")

        answered_calls = d.pop("answered_calls")

        total_duration_ms = d.pop("total_duration_ms")

        inbound_calls = d.pop("inbound_calls")

        outbound_calls = d.pop("outbound_calls")

        total_cost = d.pop("total_cost")

        usage_response = cls(
            period_start=period_start,
            period_end=period_end,
            total_calls=total_calls,
            answered_calls=answered_calls,
            total_duration_ms=total_duration_ms,
            inbound_calls=inbound_calls,
            outbound_calls=outbound_calls,
            total_cost=total_cost,
        )


        usage_response.additional_properties = d
        return usage_response

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
