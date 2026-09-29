from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="LcrRouteResponse")



@_attrs_define
class LcrRouteResponse:
    """ 
        Attributes:
            id (str):
            tenant_id (str):
            prefix (str):
            trunk_id (str):
            cost_per_minute (float):
            priority (int):
            enabled (bool):
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            description (None | str | Unset):
     """

    id: str
    tenant_id: str
    prefix: str
    trunk_id: str
    cost_per_minute: float
    priority: int
    enabled: bool
    created_at: datetime.datetime
    updated_at: datetime.datetime
    description: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        tenant_id = self.tenant_id

        prefix = self.prefix

        trunk_id = self.trunk_id

        cost_per_minute = self.cost_per_minute

        priority = self.priority

        enabled = self.enabled

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "prefix": prefix,
            "trunk_id": trunk_id,
            "cost_per_minute": cost_per_minute,
            "priority": priority,
            "enabled": enabled,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if description is not UNSET:
            field_dict["description"] = description

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        prefix = d.pop("prefix")

        trunk_id = d.pop("trunk_id")

        cost_per_minute = d.pop("cost_per_minute")

        priority = d.pop("priority")

        enabled = d.pop("enabled")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        lcr_route_response = cls(
            id=id,
            tenant_id=tenant_id,
            prefix=prefix,
            trunk_id=trunk_id,
            cost_per_minute=cost_per_minute,
            priority=priority,
            enabled=enabled,
            created_at=created_at,
            updated_at=updated_at,
            description=description,
        )


        lcr_route_response.additional_properties = d
        return lcr_route_response

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
