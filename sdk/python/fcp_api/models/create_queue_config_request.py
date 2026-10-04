from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="CreateQueueConfigRequest")



@_attrs_define
class CreateQueueConfigRequest:
    """ Create queue config request.

        Attributes:
            name (str):
            extension (str):
            strategy (None | str | Unset):
            max_wait_time_secs (int | None | Unset):
            max_queue_depth (int | None | Unset):
            overflow_destination (None | str | Unset):
            moh_class (None | str | Unset):
            announce_position (bool | None | Unset):
            announce_interval_secs (int | None | Unset):
            retry_on_no_answer (bool | None | Unset):
            agents (list[str] | None | Unset):
            priority (int | None | Unset):
            required_skills (list[str] | None | Unset):
            record_calls (bool | None | Unset):
            callback_enabled (bool | None | Unset):
            timezone (None | str | Unset):
            schedule_id (None | str | Unset): A schedule of the tenant; empty clears it.
            closed_target (None | str | Unset): A call target; empty clears it.
            agent_group_id (None | str | Unset): A group of the tenant whose members are agents too; empty clears it.
            enabled (bool | None | Unset):
     """

    name: str
    extension: str
    strategy: None | str | Unset = UNSET
    max_wait_time_secs: int | None | Unset = UNSET
    max_queue_depth: int | None | Unset = UNSET
    overflow_destination: None | str | Unset = UNSET
    moh_class: None | str | Unset = UNSET
    announce_position: bool | None | Unset = UNSET
    announce_interval_secs: int | None | Unset = UNSET
    retry_on_no_answer: bool | None | Unset = UNSET
    agents: list[str] | None | Unset = UNSET
    priority: int | None | Unset = UNSET
    required_skills: list[str] | None | Unset = UNSET
    record_calls: bool | None | Unset = UNSET
    callback_enabled: bool | None | Unset = UNSET
    timezone: None | str | Unset = UNSET
    schedule_id: None | str | Unset = UNSET
    closed_target: None | str | Unset = UNSET
    agent_group_id: None | str | Unset = UNSET
    enabled: bool | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        name = self.name

        extension = self.extension

        strategy: None | str | Unset
        if isinstance(self.strategy, Unset):
            strategy = UNSET
        else:
            strategy = self.strategy

        max_wait_time_secs: int | None | Unset
        if isinstance(self.max_wait_time_secs, Unset):
            max_wait_time_secs = UNSET
        else:
            max_wait_time_secs = self.max_wait_time_secs

        max_queue_depth: int | None | Unset
        if isinstance(self.max_queue_depth, Unset):
            max_queue_depth = UNSET
        else:
            max_queue_depth = self.max_queue_depth

        overflow_destination: None | str | Unset
        if isinstance(self.overflow_destination, Unset):
            overflow_destination = UNSET
        else:
            overflow_destination = self.overflow_destination

        moh_class: None | str | Unset
        if isinstance(self.moh_class, Unset):
            moh_class = UNSET
        else:
            moh_class = self.moh_class

        announce_position: bool | None | Unset
        if isinstance(self.announce_position, Unset):
            announce_position = UNSET
        else:
            announce_position = self.announce_position

        announce_interval_secs: int | None | Unset
        if isinstance(self.announce_interval_secs, Unset):
            announce_interval_secs = UNSET
        else:
            announce_interval_secs = self.announce_interval_secs

        retry_on_no_answer: bool | None | Unset
        if isinstance(self.retry_on_no_answer, Unset):
            retry_on_no_answer = UNSET
        else:
            retry_on_no_answer = self.retry_on_no_answer

        agents: list[str] | None | Unset
        if isinstance(self.agents, Unset):
            agents = UNSET
        elif isinstance(self.agents, list):
            agents = self.agents


        else:
            agents = self.agents

        priority: int | None | Unset
        if isinstance(self.priority, Unset):
            priority = UNSET
        else:
            priority = self.priority

        required_skills: list[str] | None | Unset
        if isinstance(self.required_skills, Unset):
            required_skills = UNSET
        elif isinstance(self.required_skills, list):
            required_skills = self.required_skills


        else:
            required_skills = self.required_skills

        record_calls: bool | None | Unset
        if isinstance(self.record_calls, Unset):
            record_calls = UNSET
        else:
            record_calls = self.record_calls

        callback_enabled: bool | None | Unset
        if isinstance(self.callback_enabled, Unset):
            callback_enabled = UNSET
        else:
            callback_enabled = self.callback_enabled

        timezone: None | str | Unset
        if isinstance(self.timezone, Unset):
            timezone = UNSET
        else:
            timezone = self.timezone

        schedule_id: None | str | Unset
        if isinstance(self.schedule_id, Unset):
            schedule_id = UNSET
        else:
            schedule_id = self.schedule_id

        closed_target: None | str | Unset
        if isinstance(self.closed_target, Unset):
            closed_target = UNSET
        else:
            closed_target = self.closed_target

        agent_group_id: None | str | Unset
        if isinstance(self.agent_group_id, Unset):
            agent_group_id = UNSET
        else:
            agent_group_id = self.agent_group_id

        enabled: bool | None | Unset
        if isinstance(self.enabled, Unset):
            enabled = UNSET
        else:
            enabled = self.enabled


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "name": name,
            "extension": extension,
        })
        if strategy is not UNSET:
            field_dict["strategy"] = strategy
        if max_wait_time_secs is not UNSET:
            field_dict["max_wait_time_secs"] = max_wait_time_secs
        if max_queue_depth is not UNSET:
            field_dict["max_queue_depth"] = max_queue_depth
        if overflow_destination is not UNSET:
            field_dict["overflow_destination"] = overflow_destination
        if moh_class is not UNSET:
            field_dict["moh_class"] = moh_class
        if announce_position is not UNSET:
            field_dict["announce_position"] = announce_position
        if announce_interval_secs is not UNSET:
            field_dict["announce_interval_secs"] = announce_interval_secs
        if retry_on_no_answer is not UNSET:
            field_dict["retry_on_no_answer"] = retry_on_no_answer
        if agents is not UNSET:
            field_dict["agents"] = agents
        if priority is not UNSET:
            field_dict["priority"] = priority
        if required_skills is not UNSET:
            field_dict["required_skills"] = required_skills
        if record_calls is not UNSET:
            field_dict["record_calls"] = record_calls
        if callback_enabled is not UNSET:
            field_dict["callback_enabled"] = callback_enabled
        if timezone is not UNSET:
            field_dict["timezone"] = timezone
        if schedule_id is not UNSET:
            field_dict["schedule_id"] = schedule_id
        if closed_target is not UNSET:
            field_dict["closed_target"] = closed_target
        if agent_group_id is not UNSET:
            field_dict["agent_group_id"] = agent_group_id
        if enabled is not UNSET:
            field_dict["enabled"] = enabled

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        name = d.pop("name")

        extension = d.pop("extension")

        def _parse_strategy(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        strategy = _parse_strategy(d.pop("strategy", UNSET))


        def _parse_max_wait_time_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        max_wait_time_secs = _parse_max_wait_time_secs(d.pop("max_wait_time_secs", UNSET))


        def _parse_max_queue_depth(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        max_queue_depth = _parse_max_queue_depth(d.pop("max_queue_depth", UNSET))


        def _parse_overflow_destination(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        overflow_destination = _parse_overflow_destination(d.pop("overflow_destination", UNSET))


        def _parse_moh_class(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        moh_class = _parse_moh_class(d.pop("moh_class", UNSET))


        def _parse_announce_position(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        announce_position = _parse_announce_position(d.pop("announce_position", UNSET))


        def _parse_announce_interval_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        announce_interval_secs = _parse_announce_interval_secs(d.pop("announce_interval_secs", UNSET))


        def _parse_retry_on_no_answer(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        retry_on_no_answer = _parse_retry_on_no_answer(d.pop("retry_on_no_answer", UNSET))


        def _parse_agents(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                agents_type_0 = cast(list[str], data)

                return agents_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        agents = _parse_agents(d.pop("agents", UNSET))


        def _parse_priority(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        priority = _parse_priority(d.pop("priority", UNSET))


        def _parse_required_skills(data: object) -> list[str] | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, list):
                    raise TypeError()
                required_skills_type_0 = cast(list[str], data)

                return required_skills_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(list[str] | None | Unset, data)

        required_skills = _parse_required_skills(d.pop("required_skills", UNSET))


        def _parse_record_calls(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        record_calls = _parse_record_calls(d.pop("record_calls", UNSET))


        def _parse_callback_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        callback_enabled = _parse_callback_enabled(d.pop("callback_enabled", UNSET))


        def _parse_timezone(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        timezone = _parse_timezone(d.pop("timezone", UNSET))


        def _parse_schedule_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        schedule_id = _parse_schedule_id(d.pop("schedule_id", UNSET))


        def _parse_closed_target(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        closed_target = _parse_closed_target(d.pop("closed_target", UNSET))


        def _parse_agent_group_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        agent_group_id = _parse_agent_group_id(d.pop("agent_group_id", UNSET))


        def _parse_enabled(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        enabled = _parse_enabled(d.pop("enabled", UNSET))


        create_queue_config_request = cls(
            name=name,
            extension=extension,
            strategy=strategy,
            max_wait_time_secs=max_wait_time_secs,
            max_queue_depth=max_queue_depth,
            overflow_destination=overflow_destination,
            moh_class=moh_class,
            announce_position=announce_position,
            announce_interval_secs=announce_interval_secs,
            retry_on_no_answer=retry_on_no_answer,
            agents=agents,
            priority=priority,
            required_skills=required_skills,
            record_calls=record_calls,
            callback_enabled=callback_enabled,
            timezone=timezone,
            schedule_id=schedule_id,
            closed_target=closed_target,
            agent_group_id=agent_group_id,
            enabled=enabled,
        )


        create_queue_config_request.additional_properties = d
        return create_queue_config_request

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
