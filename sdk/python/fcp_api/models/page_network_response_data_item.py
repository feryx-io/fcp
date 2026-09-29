from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PageNetworkResponseDataItem")



@_attrs_define
class PageNetworkResponseDataItem:
    """ 
        Attributes:
            id (str):
            tenant_id (str):
            cidr (str):
            location_id (str):
            priority (int):
            applies_to (str): `nomadic` (the default) or `all`.
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            name (None | str | Unset):
     """

    id: str
    tenant_id: str
    cidr: str
    location_id: str
    priority: int
    applies_to: str
    created_at: datetime.datetime
    updated_at: datetime.datetime
    name: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        tenant_id = self.tenant_id

        cidr = self.cidr

        location_id = self.location_id

        priority = self.priority

        applies_to = self.applies_to

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "cidr": cidr,
            "location_id": location_id,
            "priority": priority,
            "applies_to": applies_to,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if name is not UNSET:
            field_dict["name"] = name

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        cidr = d.pop("cidr")

        location_id = d.pop("location_id")

        priority = d.pop("priority")

        applies_to = d.pop("applies_to")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


        page_network_response_data_item = cls(
            id=id,
            tenant_id=tenant_id,
            cidr=cidr,
            location_id=location_id,
            priority=priority,
            applies_to=applies_to,
            created_at=created_at,
            updated_at=updated_at,
            name=name,
        )


        page_network_response_data_item.additional_properties = d
        return page_network_response_data_item

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
