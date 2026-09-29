from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="PageQueueResponseDataItem")



@_attrs_define
class PageQueueResponseDataItem:
    """ Queue info response.

        Attributes:
            name (str):
            display_name (str):
            extension (str):
            strategy (str):
            max_wait_time_secs (int):
            max_queue_depth (int):
            agents_configured (int):
            calls_waiting (int):
            overflow_destination (None | str | Unset):
     """

    name: str
    display_name: str
    extension: str
    strategy: str
    max_wait_time_secs: int
    max_queue_depth: int
    agents_configured: int
    calls_waiting: int
    overflow_destination: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        name = self.name

        display_name = self.display_name

        extension = self.extension

        strategy = self.strategy

        max_wait_time_secs = self.max_wait_time_secs

        max_queue_depth = self.max_queue_depth

        agents_configured = self.agents_configured

        calls_waiting = self.calls_waiting

        overflow_destination: None | str | Unset
        if isinstance(self.overflow_destination, Unset):
            overflow_destination = UNSET
        else:
            overflow_destination = self.overflow_destination


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "name": name,
            "display_name": display_name,
            "extension": extension,
            "strategy": strategy,
            "max_wait_time_secs": max_wait_time_secs,
            "max_queue_depth": max_queue_depth,
            "agents_configured": agents_configured,
            "calls_waiting": calls_waiting,
        })
        if overflow_destination is not UNSET:
            field_dict["overflow_destination"] = overflow_destination

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        name = d.pop("name")

        display_name = d.pop("display_name")

        extension = d.pop("extension")

        strategy = d.pop("strategy")

        max_wait_time_secs = d.pop("max_wait_time_secs")

        max_queue_depth = d.pop("max_queue_depth")

        agents_configured = d.pop("agents_configured")

        calls_waiting = d.pop("calls_waiting")

        def _parse_overflow_destination(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        overflow_destination = _parse_overflow_destination(d.pop("overflow_destination", UNSET))


        page_queue_response_data_item = cls(
            name=name,
            display_name=display_name,
            extension=extension,
            strategy=strategy,
            max_wait_time_secs=max_wait_time_secs,
            max_queue_depth=max_queue_depth,
            agents_configured=agents_configured,
            calls_waiting=calls_waiting,
            overflow_destination=overflow_destination,
        )


        page_queue_response_data_item.additional_properties = d
        return page_queue_response_data_item

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
