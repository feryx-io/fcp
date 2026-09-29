from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="ReadinessSummary")



@_attrs_define
class ReadinessSummary:
    """ 
        Attributes:
            users (int):
            ready (int):
            missing_location (int):
            missing_callback (int):
            unvalidated_address (int):
            unplaced_nomadic (int): Users whose devices all move but whom nothing places where they are.
     """

    users: int
    ready: int
    missing_location: int
    missing_callback: int
    unvalidated_address: int
    unplaced_nomadic: int
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        users = self.users

        ready = self.ready

        missing_location = self.missing_location

        missing_callback = self.missing_callback

        unvalidated_address = self.unvalidated_address

        unplaced_nomadic = self.unplaced_nomadic


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "users": users,
            "ready": ready,
            "missing_location": missing_location,
            "missing_callback": missing_callback,
            "unvalidated_address": unvalidated_address,
            "unplaced_nomadic": unplaced_nomadic,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        users = d.pop("users")

        ready = d.pop("ready")

        missing_location = d.pop("missing_location")

        missing_callback = d.pop("missing_callback")

        unvalidated_address = d.pop("unvalidated_address")

        unplaced_nomadic = d.pop("unplaced_nomadic")

        readiness_summary = cls(
            users=users,
            ready=ready,
            missing_location=missing_location,
            missing_callback=missing_callback,
            unvalidated_address=unvalidated_address,
            unplaced_nomadic=unplaced_nomadic,
        )


        readiness_summary.additional_properties = d
        return readiness_summary

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
