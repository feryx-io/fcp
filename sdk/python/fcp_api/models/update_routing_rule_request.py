from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="UpdateRoutingRuleRequest")



@_attrs_define
class UpdateRoutingRuleRequest:
    """ 
        Attributes:
            name (None | str | Unset):
            description (None | str | Unset):
            priority (int | None | Unset):
            enabled (bool | None | Unset):
            destination_pattern (None | str | Unset):
            source_pattern (None | str | Unset):
            source_ip_ranges (list[str] | None | Unset):
            time_window_days (list[str] | None | Unset):
            time_window_start (None | str | Unset):
            time_window_end (None | str | Unset):
            time_window_timezone (None | str | Unset):
            action (None | str | Unset):
            target_trunk_id (None | str | Unset):
            target_trunk_group (None | str | Unset):
            reason (None | str | Unset):
     """

    name: None | str | Unset = UNSET
    description: None | str | Unset = UNSET
    priority: int | None | Unset = UNSET
    enabled: bool | None | Unset = UNSET
    destination_pattern: None | str | Unset = UNSET
    source_pattern: None | str | Unset = UNSET
    source_ip_ranges: list[str] | None | Unset = UNSET
    time_window_days: list[str] | None | Unset = UNSET
    time_window_start: None | str | Unset = UNSET
    time_window_end: None | str | Unset = UNSET
    time_window_timezone: None | str | Unset = UNSET
    action: None | str | Unset = UNSET
    target_trunk_id: None | str | Unset = UNSET
    target_trunk_group: None | str | Unset = UNSET
    reason: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        name: None | str | Unset
        if isinstance(self.name, Unset):
            name = UNSET
        else:
            name = self.name

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

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

        source_ip_ranges: list[str] | None | Unset
        if isinstance(self.source_ip_ranges, Unset):
            source_ip_ranges = UNSET
        elif isinstance(self.source_ip_ranges, list):
            source_ip_ranges = self.source_ip_ranges


        else:
            source_ip_ranges = self.source_ip_ranges

        time_window_days: list[str] | None | Unset
        if isinstance(self.time_window_days, Unset):
            time_window_days = UNSET
        elif isinstance(self.time_window_days, list):
            time_window_days = self.time_window_days


        else:
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

        time_window_timezone: None | str | Unset
        if isinstance(self.time_window_timezone, Unset):
            time_window_timezone = UNSET
        else:
            time_window_timezone = self.time_window_timezone

        action: None | str | Unset
        if isinstance(self.action, Unset):
            action = UNSET
        else:
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
        })
        if name is not UNSET:
            field_dict["name"] = name
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
        def _parse_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        name = _parse_name(d.pop("name", UNSET))


        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


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


        def _parse_source_ip_ranges(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                source_ip_ranges_type_0 = cast(list[str], data)

                return source_ip_ranges_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        source_ip_ranges = _parse_source_ip_ranges(d.pop("source_ip_ranges", UNSET))


        def _parse_time_window_days(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                time_window_days_type_0 = cast(list[str], data)

                return time_window_days_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        time_window_days = _parse_time_window_days(d.pop("time_window_days", UNSET))


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


        def _parse_time_window_timezone(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        time_window_timezone = _parse_time_window_timezone(d.pop("time_window_timezone", UNSET))


        def _parse_action(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        action = _parse_action(d.pop("action", UNSET))


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


        update_routing_rule_request = cls(
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


        update_routing_rule_request.additional_properties = d
        return update_routing_rule_request

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
