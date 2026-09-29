from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="StatusResponse")



@_attrs_define
class StatusResponse:
    """ System status response.

        Attributes:
            status (str):
            version (str):
            uptime_seconds (int):
            users_count (int):
            active_registrations_count (int):
     """

    status: str
    version: str
    uptime_seconds: int
    users_count: int
    active_registrations_count: int
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        status = self.status

        version = self.version

        uptime_seconds = self.uptime_seconds

        users_count = self.users_count

        active_registrations_count = self.active_registrations_count


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "status": status,
            "version": version,
            "uptime_seconds": uptime_seconds,
            "users_count": users_count,
            "active_registrations_count": active_registrations_count,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        status = d.pop("status")

        version = d.pop("version")

        uptime_seconds = d.pop("uptime_seconds")

        users_count = d.pop("users_count")

        active_registrations_count = d.pop("active_registrations_count")

        status_response = cls(
            status=status,
            version=version,
            uptime_seconds=uptime_seconds,
            users_count=users_count,
            active_registrations_count=active_registrations_count,
        )


        status_response.additional_properties = d
        return status_response

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
