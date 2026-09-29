from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="DashboardSummary")



@_attrs_define
class DashboardSummary:
    """ 
        Attributes:
            active_calls (int):
            registered_devices (int):
            total_users (int):
            total_extensions (int):
            total_phone_numbers (int):
            calls_today (int):
            avg_call_duration_secs (float):
     """

    active_calls: int
    registered_devices: int
    total_users: int
    total_extensions: int
    total_phone_numbers: int
    calls_today: int
    avg_call_duration_secs: float
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        active_calls = self.active_calls

        registered_devices = self.registered_devices

        total_users = self.total_users

        total_extensions = self.total_extensions

        total_phone_numbers = self.total_phone_numbers

        calls_today = self.calls_today

        avg_call_duration_secs = self.avg_call_duration_secs


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "active_calls": active_calls,
            "registered_devices": registered_devices,
            "total_users": total_users,
            "total_extensions": total_extensions,
            "total_phone_numbers": total_phone_numbers,
            "calls_today": calls_today,
            "avg_call_duration_secs": avg_call_duration_secs,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        active_calls = d.pop("active_calls")

        registered_devices = d.pop("registered_devices")

        total_users = d.pop("total_users")

        total_extensions = d.pop("total_extensions")

        total_phone_numbers = d.pop("total_phone_numbers")

        calls_today = d.pop("calls_today")

        avg_call_duration_secs = d.pop("avg_call_duration_secs")

        dashboard_summary = cls(
            active_calls=active_calls,
            registered_devices=registered_devices,
            total_users=total_users,
            total_extensions=total_extensions,
            total_phone_numbers=total_phone_numbers,
            calls_today=calls_today,
            avg_call_duration_secs=avg_call_duration_secs,
        )


        dashboard_summary.additional_properties = d
        return dashboard_summary

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
