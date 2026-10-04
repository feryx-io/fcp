from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PageDeviceFetchResponseDataItem")



@_attrs_define
class PageDeviceFetchResponseDataItem:
    """ One fetch of a device's file.

        Attributes:
            file (str):
            at (datetime.datetime):
            outcome (str): `served`, `unauthorized`, `not_served` (disabled, or no template)…
            address (None | str | Unset):
            user_agent (None | str | Unset):
     """

    file: str
    at: datetime.datetime
    outcome: str
    address: None | str | Unset = UNSET
    user_agent: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        file = self.file

        at = self.at.isoformat()

        outcome = self.outcome

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


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "file": file,
            "at": at,
            "outcome": outcome,
        })
        if address is not UNSET:
            field_dict["address"] = address
        if user_agent is not UNSET:
            field_dict["user_agent"] = user_agent

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        file = d.pop("file")

        at = datetime.datetime.fromisoformat(d.pop("at"))




        outcome = d.pop("outcome")

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


        page_device_fetch_response_data_item = cls(
            file=file,
            at=at,
            outcome=outcome,
            address=address,
            user_agent=user_agent,
        )


        page_device_fetch_response_data_item.additional_properties = d
        return page_device_fetch_response_data_item

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
