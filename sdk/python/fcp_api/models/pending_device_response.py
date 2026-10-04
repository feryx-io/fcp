from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PendingDeviceResponse")



@_attrs_define
class PendingDeviceResponse:
    """ A phone that fetched with a MAC no device has.

        Attributes:
            mac_address (str):
            first_seen (datetime.datetime):
            last_seen (datetime.datetime):
            fetches (int):
            address (None | str | Unset):
            user_agent (None | str | Unset):
            vendor (None | str | Unset):
            model (None | str | Unset):
            firmware (None | str | Unset):
     """

    mac_address: str
    first_seen: datetime.datetime
    last_seen: datetime.datetime
    fetches: int
    address: None | str | Unset = UNSET
    user_agent: None | str | Unset = UNSET
    vendor: None | str | Unset = UNSET
    model: None | str | Unset = UNSET
    firmware: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        mac_address = self.mac_address

        first_seen = self.first_seen.isoformat()

        last_seen = self.last_seen.isoformat()

        fetches = self.fetches

        address: None | str | Unset
        if isinstance(self.address, Unset):
            address = UNSET
        else:
            address = self.address

        user_agent: None | str | Unset
        if isinstance(self.user_agent, Unset):
            user_agent = UNSET
        else:
            user_agent = self.user_agent

        vendor: None | str | Unset
        if isinstance(self.vendor, Unset):
            vendor = UNSET
        else:
            vendor = self.vendor

        model: None | str | Unset
        if isinstance(self.model, Unset):
            model = UNSET
        else:
            model = self.model

        firmware: None | str | Unset
        if isinstance(self.firmware, Unset):
            firmware = UNSET
        else:
            firmware = self.firmware


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "mac_address": mac_address,
            "first_seen": first_seen,
            "last_seen": last_seen,
            "fetches": fetches,
        })
        if address is not UNSET:
            field_dict["address"] = address
        if user_agent is not UNSET:
            field_dict["user_agent"] = user_agent
        if vendor is not UNSET:
            field_dict["vendor"] = vendor
        if model is not UNSET:
            field_dict["model"] = model
        if firmware is not UNSET:
            field_dict["firmware"] = firmware

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        mac_address = d.pop("mac_address")

        first_seen = datetime.datetime.fromisoformat(d.pop("first_seen"))




        last_seen = datetime.datetime.fromisoformat(d.pop("last_seen"))




        fetches = d.pop("fetches")

        def _parse_address(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        address = _parse_address(d.pop("address", UNSET))


        def _parse_user_agent(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        user_agent = _parse_user_agent(d.pop("user_agent", UNSET))


        def _parse_vendor(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        vendor = _parse_vendor(d.pop("vendor", UNSET))


        def _parse_model(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        model = _parse_model(d.pop("model", UNSET))


        def _parse_firmware(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        firmware = _parse_firmware(d.pop("firmware", UNSET))


        pending_device_response = cls(
            mac_address=mac_address,
            first_seen=first_seen,
            last_seen=last_seen,
            fetches=fetches,
            address=address,
            user_agent=user_agent,
            vendor=vendor,
            model=model,
            firmware=firmware,
        )


        pending_device_response.additional_properties = d
        return pending_device_response

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
