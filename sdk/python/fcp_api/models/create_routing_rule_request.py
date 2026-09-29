from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="CreateRoutingRuleRequest")



@_attrs_define
class CreateRoutingRuleRequest:
    """ 
        Attributes:
            name (str):
            description (None | str | Unset):
            priority (int | Unset):
            enabled (bool | Unset):
            destination_pattern (None | str | Unset):
            source_pattern (None | str | Unset):
            source_ip_ranges (list[str] | Unset):
            time_window_days (list[str] | Unset):
            time_window_start (None | str | Unset):
            time_window_end (None | str | Unset):
            time_window_timezone (str | Unset):
            action (str | Unset):
            target_trunk_id (None | str | Unset):
            target_trunk_group (None | str | Unset):
            reason (None | str | Unset):
     """

    name: str
    description: None | str | Unset = UNSET
    priority: int | Unset = UNSET
    enabled: bool | Unset = UNSET
    destination_pattern: None | str | Unset = UNSET
    source_pattern: None | str | Unset = UNSET
    source_ip_ranges: list[str] | Unset = UNSET
    time_window_days: list[str] | Unset = UNSET
    time_window_start: None | str | Unset = UNSET
    time_window_end: None | str | Unset = UNSET
    time_window_timezone: str | Unset = UNSET
    action: str | Unset = UNSET
    target_trunk_id: None | str | Unset = UNSET
    target_trunk_group: None | str | Unset = UNSET
    reason: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        name = self.name

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

        priority = self.priority

        enabled = self.enabled

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

        source_ip_ranges: list[str] | Unset = UNSET
        if not isinstance(self.source_ip_ranges, Unset):
            source_ip_ranges = self.source_ip_ranges



        time_window_days: list[str] | Unset = UNSET
        if not isinstance(self.time_window_days, Unset):
            time_window_days = self.time_window_days



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

        time_window_timezone = self.time_window_timezone

        action = self.action

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
            "name": name,
        })
        if description is not UNSET:
            field_dict["description"] = description
        if priority is not UNSET:
            field_dict["priority"] = priority
        if enabled is not UNSET:
            field_dict["enabled"] = enabled
        if destination_pattern is not UNSET:
            field_dict["destination_pattern"] = destination_pattern
        if source_pattern is not UNSET:
            field_dict["source_pattern"] = source_pattern
        if source_ip_ranges is not UNSET:
            field_dict["source_ip_ranges"] = source_ip_ranges
        if time_window_days is not UNSET:
            field_dict["time_window_days"] = time_window_days
        if time_window_start is not UNSET:
            field_dict["time_window_start"] = time_window_start
        if time_window_end is not UNSET:
            field_dict["time_window_end"] = time_window_end
        if time_window_timezone is not UNSET:
            field_dict["time_window_timezone"] = time_window_timezone
        if action is not UNSET:
            field_dict["action"] = action
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
        name = d.pop("name")

        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        priority = d.pop("priority", UNSET)

        enabled = d.pop("enabled", UNSET)

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


        source_ip_ranges = cast(list[str], d.pop("source_ip_ranges", UNSET))


        time_window_days = cast(list[str], d.pop("time_window_days", UNSET))


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


        time_window_timezone = d.pop("time_window_timezone", UNSET)

        action = d.pop("action", UNSET)

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


        create_routing_rule_request = cls(
            name=name,
            description=description,
            priority=priority,
            enabled=enabled,
            destination_pattern=destination_pattern,
            source_pattern=source_pattern,
            source_ip_ranges=source_ip_ranges,
            time_window_days=time_window_days,
            time_window_start=time_window_start,
            time_window_end=time_window_end,
            time_window_timezone=time_window_timezone,
            action=action,
            target_trunk_id=target_trunk_id,
            target_trunk_group=target_trunk_group,
            reason=reason,
        )


        create_routing_rule_request.additional_properties = d
        return create_routing_rule_request

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
