from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="CreateNetworkRequest")



@_attrs_define
class CreateNetworkRequest:
    """ 
        Attributes:
            cidr (str): A range (`10.1.2.0/24`) or a single address.
            location_id (str): The site an address on this network is at.
            name (None | str | Unset):
            priority (int | None | Unset):
            applies_to (None | str | Unset): `nomadic` (the default) or `all`.
     """

    cidr: str
    location_id: str
    name: None | str | Unset = UNSET
    priority: int | None | Unset = UNSET
    applies_to: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        cidr = self.cidr

        location_id = self.location_id

        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name

        priority: int | None | Unset
        if isinstance(self.priority, Unset):
            priority = UNSET
        else:
            priority = self.priority

        applies_to: None | str | Unset
        if isinstance(self.applies_to, Unset):
            applies_to = UNSET
        else:
            applies_to = self.applies_to


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "cidr": cidr,
            "location_id": location_id,
        })
        if name is not UNSET:
            field_dict["name"] = name
        if priority is not UNSET:
            field_dict["priority"] = priority
        if applies_to is not UNSET:
            field_dict["applies_to"] = applies_to

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        cidr = d.pop("cidr")

        location_id = d.pop("location_id")

        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


        def _parse_priority(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        priority = _parse_priority(d.pop("priority", UNSET))


        def _parse_applies_to(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        applies_to = _parse_applies_to(d.pop("applies_to", UNSET))


        create_network_request = cls(
            cidr=cidr,
            location_id=location_id,
            name=name,
            priority=priority,
            applies_to=applies_to,
        )


        create_network_request.additional_properties = d
        return create_network_request

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
