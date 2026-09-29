from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="RoutingRuleResponse")



@_attrs_define
class RoutingRuleResponse:
    """ 
        Attributes:
            id (str):
            tenant_id (str):
            name (str):
            priority (int):
            enabled (bool):
            source_ip_ranges (list[str]):
            time_window_days (list[str]):
            time_window_timezone (str):
            action (str):
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            description (None | str | Unset):
            destination_pattern (None | str | Unset):
            source_pattern (None | str | Unset):
            time_window_start (None | str | Unset):
            time_window_end (None | str | Unset):
            target_trunk_id (None | str | Unset):
            target_trunk_group (None | str | Unset):
            reason (None | str | Unset):
     """

    id: str
    tenant_id: str
    name: str
    priority: int
    enabled: bool
    source_ip_ranges: list[str]
    time_window_days: list[str]
    time_window_timezone: str
    action: str
    created_at: datetime.datetime
    updated_at: datetime.datetime
    description: None | str | Unset = UNSET
    destination_pattern: None | str | Unset = UNSET
    source_pattern: None | str | Unset = UNSET
    time_window_start: None | str | Unset = UNSET
    time_window_end: None | str | Unset = UNSET
    target_trunk_id: None | str | Unset = UNSET
    target_trunk_group: None | str | Unset = UNSET
    reason: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        tenant_id = self.tenant_id

        name = self.name

        priority = self.priority

        enabled = self.enabled

        source_ip_ranges = self.source_ip_ranges



        time_window_days = self.time_window_days



        time_window_timezone = self.time_window_timezone

        action = self.action

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

        destination_pattern: None | str | Unset
        if isinstance(self.destination_pattern, Unset):
            destination_pattern = UNSET
        else:
            destination_pattern = self.destination_pattern

        source_pattern: None | str | Unset
        if isinstance(self.source_pattern, Unset):
            source_pattern = UNSET
        else:
            source_pattern = self.source_pattern

        time_window_start: None | str | Unset
        if isinstance(self.time_window_start, Unset):
            time_window_start = UNSET
        else:
            time_window_start = self.time_window_start

        time_window_end: None | str | Unset
        if isinstance(self.time_window_end, Unset):
            time_window_end = UNSET
        else:
            time_window_end = self.time_window_end

        target_trunk_id: None | str | Unset
        if isinstance(self.target_trunk_id, Unset):
            target_trunk_id = UNSET
        else:
            target_trunk_id = self.target_trunk_id

        target_trunk_group: None | str | Unset
        if isinstance(self.target_trunk_group, Unset):
            target_trunk_group = UNSET
        else:
            target_trunk_group = self.target_trunk_group

        reason: None | str | Unset
        if isinstance(self.reason, Unset):
            reason = UNSET
        else:
            reason = self.reason


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "tenant_id": tenant_id,
            "name": name,
            "priority": priority,
            "enabled": enabled,
            "source_ip_ranges": source_ip_ranges,
            "time_window_days": time_window_days,
            "time_window_timezone": time_window_timezone,
            "action": action,
            "created_at": created_at,
            "updated_at": updated_at,
        })
        if description is not UNSET:
            field_dict["description"] = description
        if destination_pattern is not UNSET:
            field_dict["destination_pattern"] = destination_pattern
        if source_pattern is not UNSET:
            field_dict["source_pattern"] = source_pattern
        if time_window_start is not UNSET:
            field_dict["time_window_start"] = time_window_start
        if time_window_end is not UNSET:
            field_dict["time_window_end"] = time_window_end
        if target_trunk_id is not UNSET:
            field_dict["target_trunk_id"] = target_trunk_id
        if target_trunk_group is not UNSET:
            field_dict["target_trunk_group"] = target_trunk_group
        if reason is not UNSET:
            field_dict["reason"] = reason

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        tenant_id = d.pop("tenant_id")

        name = d.pop("name")

        priority = d.pop("priority")

        enabled = d.pop("enabled")

        source_ip_ranges = cast(list[str], d.pop("source_ip_ranges"))


        time_window_days = cast(list[str], d.pop("time_window_days"))


        time_window_timezone = d.pop("time_window_timezone")

        action = d.pop("action")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        def _parse_destination_pattern(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        destination_pattern = _parse_destination_pattern(d.pop("destination_pattern", UNSET))


        def _parse_source_pattern(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        source_pattern = _parse_source_pattern(d.pop("source_pattern", UNSET))


        def _parse_time_window_start(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        time_window_start = _parse_time_window_start(d.pop("time_window_start", UNSET))


        def _parse_time_window_end(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        time_window_end = _parse_time_window_end(d.pop("time_window_end", UNSET))


        def _parse_target_trunk_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        target_trunk_id = _parse_target_trunk_id(d.pop("target_trunk_id", UNSET))


        def _parse_target_trunk_group(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        target_trunk_group = _parse_target_trunk_group(d.pop("target_trunk_group", UNSET))


        def _parse_reason(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        reason = _parse_reason(d.pop("reason", UNSET))


        routing_rule_response = cls(
            id=id,
            tenant_id=tenant_id,
            name=name,
            priority=priority,
            enabled=enabled,
            source_ip_ranges=source_ip_ranges,
            time_window_days=time_window_days,
            time_window_timezone=time_window_timezone,
            action=action,
            created_at=created_at,
            updated_at=updated_at,
            description=description,
            destination_pattern=destination_pattern,
            source_pattern=source_pattern,
            time_window_start=time_window_start,
            time_window_end=time_window_end,
            target_trunk_id=target_trunk_id,
            target_trunk_group=target_trunk_group,
            reason=reason,
        )


        routing_rule_response.additional_properties = d
        return routing_rule_response

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
