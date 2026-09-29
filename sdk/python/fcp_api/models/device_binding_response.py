from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="DeviceBindingResponse")



@_attrs_define
class DeviceBindingResponse:
    """ 
        Attributes:
            device_id (str):
            priority (int):
            line_appearance (int):
            shared_line (bool):
            enabled (bool):
     """

    device_id: str
    priority: int
    line_appearance: int
    shared_line: bool
    enabled: bool
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        device_id = self.device_id

        priority = self.priority

        line_appearance = self.line_appearance

        shared_line = self.shared_line

        enabled = self.enabled


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "device_id": device_id,
            "priority": priority,
            "line_appearance": line_appearance,
            "shared_line": shared_line,
            "enabled": enabled,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        device_id = d.pop("device_id")

        priority = d.pop("priority")

        line_appearance = d.pop("line_appearance")

        shared_line = d.pop("shared_line")

        enabled = d.pop("enabled")

        device_binding_response = cls(
            device_id=device_id,
            priority=priority,
            line_appearance=line_appearance,
            shared_line=shared_line,
            enabled=enabled,
        )


        device_binding_response.additional_properties = d
        return device_binding_response

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
