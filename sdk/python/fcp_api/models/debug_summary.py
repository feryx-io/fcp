from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="DebugSummary")



@_attrs_define
class DebugSummary:
    """ System debug summary.

        Attributes:
            active_dialogs (int):
            active_subscriptions (int):
            active_calls (int):
            rate_limit_entries (int):
     """

    active_dialogs: int
    active_subscriptions: int
    active_calls: int
    rate_limit_entries: int
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        active_dialogs = self.active_dialogs

        active_subscriptions = self.active_subscriptions

        active_calls = self.active_calls

        rate_limit_entries = self.rate_limit_entries


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "active_dialogs": active_dialogs,
            "active_subscriptions": active_subscriptions,
            "active_calls": active_calls,
            "rate_limit_entries": rate_limit_entries,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        active_dialogs = d.pop("active_dialogs")

        active_subscriptions = d.pop("active_subscriptions")

        active_calls = d.pop("active_calls")

        rate_limit_entries = d.pop("rate_limit_entries")

        debug_summary = cls(
            active_dialogs=active_dialogs,
            active_subscriptions=active_subscriptions,
            active_calls=active_calls,
            rate_limit_entries=rate_limit_entries,
        )


        debug_summary.additional_properties = d
        return debug_summary

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
