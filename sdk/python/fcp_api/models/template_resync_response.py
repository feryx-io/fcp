from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="TemplateResyncResponse")



@_attrs_define
class TemplateResyncResponse:
    """ What a template's resync did.

        Attributes:
            devices (int): The tenant's devices given this template (named, or picked by vendor
                and model), enabled.
            sent (int): NOTIFYs sent: one per registered contact.
     """

    devices: int
    sent: int
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        devices = self.devices

        sent = self.sent


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "devices": devices,
            "sent": sent,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        devices = d.pop("devices")

        sent = d.pop("sent")

        template_resync_response = cls(
            devices=devices,
            sent=sent,
        )


        template_resync_response.additional_properties = d
        return template_resync_response

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
