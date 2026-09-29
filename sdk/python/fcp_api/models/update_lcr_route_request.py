from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="UpdateLcrRouteRequest")



@_attrs_define
class UpdateLcrRouteRequest:
    """ 
        Attributes:
            prefix (None | str | Unset):
            trunk_id (None | str | Unset):
            cost_per_minute (float | None | Unset):
            priority (int | None | Unset):
            enabled (bool | None | Unset):
            description (None | str | Unset): An empty string clears it.
     """

    prefix: None | str | Unset = UNSET
    trunk_id: None | str | Unset = UNSET
    cost_per_minute: float | None | Unset = UNSET
    priority: int | None | Unset = UNSET
    enabled: bool | None | Unset = UNSET
    description: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        prefix: None | str | Unset
        if isinstance(self.prefix, Unset):
            prefix = UNSET
        else:
            prefix = self.prefix

        trunk_id: None | str | Unset
        if isinstance(self.trunk_id, Unset):
            trunk_id = UNSET
        else:
            trunk_id = self.trunk_id

        cost_per_minute: float | None | Unset
        if isinstance(self.cost_per_minute, Unset):
            cost_per_minute = UNSET
        else:
            cost_per_minute = self.cost_per_minute

        priority: int | None | Unset
        if isinstance(self.priority, Unset):
            priority = UNSET
        else:
            priority = self.priority

        enabled: bool | None | Unset
        if isinstance(self.enabled, Unset):
            enabled = UNSET
        else:
            enabled = self.enabled

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if prefix is not UNSET:
            field_dict["prefix"] = prefix
        if trunk_id is not UNSET:
            field_dict["trunk_id"] = trunk_id
        if cost_per_minute is not UNSET:
            field_dict["cost_per_minute"] = cost_per_minute
        if priority is not UNSET:
            field_dict["priority"] = priority
        if enabled is not UNSET:
            field_dict["enabled"] = enabled
        if description is not UNSET:
            field_dict["description"] = description

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        def _parse_prefix(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        prefix = _parse_prefix(d.pop("prefix", UNSET))


        def _parse_trunk_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        trunk_id = _parse_trunk_id(d.pop("trunk_id", UNSET))


        def _parse_cost_per_minute(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        cost_per_minute = _parse_cost_per_minute(d.pop("cost_per_minute", UNSET))


        def _parse_priority(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        priority = _parse_priority(d.pop("priority", UNSET))


        def _parse_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        enabled = _parse_enabled(d.pop("enabled", UNSET))


        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        update_lcr_route_request = cls(
            prefix=prefix,
            trunk_id=trunk_id,
            cost_per_minute=cost_per_minute,
            priority=priority,
            enabled=enabled,
            description=description,
        )


        update_lcr_route_request.additional_properties = d
        return update_lcr_route_request

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
