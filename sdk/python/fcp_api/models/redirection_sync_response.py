from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset







T = TypeVar("T", bound="RedirectionSyncResponse")



@_attrs_define
class RedirectionSyncResponse:
    """ What a sync did.

        Attributes:
            devices (int): The tenant's enabled devices of the service's vendor with a MAC.
            registered (int):
            failed (int):
     """

    devices: int
    registered: int
    failed: int
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        devices = self.devices

        registered = self.registered

        failed = self.failed


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "devices": devices,
            "registered": registered,
            "failed": failed,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        devices = d.pop("devices")

        registered = d.pop("registered")

        failed = d.pop("failed")

        redirection_sync_response = cls(
            devices=devices,
            registered=registered,
            failed=failed,
        )


        redirection_sync_response.additional_properties = d
        return redirection_sync_response

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
