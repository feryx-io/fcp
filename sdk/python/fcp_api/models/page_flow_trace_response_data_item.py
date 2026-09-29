from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PageFlowTraceResponseDataItem")



@_attrs_define
class PageFlowTraceResponseDataItem:
    """ Flow trace response model.

        Attributes:
            id (int):
            call_id (str):
            flow_id (str):
            event_type (str):
            timestamp (datetime.datetime):
            flow_name (None | str | Unset):
            node_id (None | str | Unset):
            detail (None | str | Unset):
            flow_version (int | None | Unset):
            elapsed_ms (int | None | Unset):
     """

    id: int
    call_id: str
    flow_id: str
    event_type: str
    timestamp: datetime.datetime
    flow_name: None | str | Unset = UNSET
    node_id: None | str | Unset = UNSET
    detail: None | str | Unset = UNSET
    flow_version: int | None | Unset = UNSET
    elapsed_ms: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        call_id = self.call_id

        flow_id = self.flow_id

        event_type = self.event_type

        timestamp = self.timestamp.isoformat()

        flow_name: None | str | Unset
        if isinstance(self.flow_name, Unset):
            flow_name = UNSET
        else:
            flow_name = self.flow_name

        node_id: None | str | Unset
        if isinstance(self.node_id, Unset):
            node_id = UNSET
        else:
            node_id = self.node_id

        detail: None | str | Unset
        if isinstance(self.detail, Unset):
            detail = UNSET
        else:
            detail = self.detail

        flow_version: int | None | Unset
        if isinstance(self.flow_version, Unset):
            flow_version = UNSET
        else:
            flow_version = self.flow_version

        elapsed_ms: int | None | Unset
        if isinstance(self.elapsed_ms, Unset):
            elapsed_ms = UNSET
        else:
            elapsed_ms = self.elapsed_ms


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "call_id": call_id,
            "flow_id": flow_id,
            "event_type": event_type,
            "timestamp": timestamp,
        })
        if flow_name is not UNSET:
            field_dict["flow_name"] = flow_name
        if node_id is not UNSET:
            field_dict["node_id"] = node_id
        if detail is not UNSET:
            field_dict["detail"] = detail
        if flow_version is not UNSET:
            field_dict["flow_version"] = flow_version
        if elapsed_ms is not UNSET:
            field_dict["elapsed_ms"] = elapsed_ms

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        call_id = d.pop("call_id")

        flow_id = d.pop("flow_id")

        event_type = d.pop("event_type")

        timestamp = datetime.datetime.fromisoformat(d.pop("timestamp"))




        def _parse_flow_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        flow_name = _parse_flow_name(d.pop("flow_name", UNSET))


        def _parse_node_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        node_id = _parse_node_id(d.pop("node_id", UNSET))


        def _parse_detail(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        detail = _parse_detail(d.pop("detail", UNSET))


        def _parse_flow_version(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        flow_version = _parse_flow_version(d.pop("flow_version", UNSET))


        def _parse_elapsed_ms(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        elapsed_ms = _parse_elapsed_ms(d.pop("elapsed_ms", UNSET))


        page_flow_trace_response_data_item = cls(
            id=id,
            call_id=call_id,
            flow_id=flow_id,
            event_type=event_type,
            timestamp=timestamp,
            flow_name=flow_name,
            node_id=node_id,
            detail=detail,
            flow_version=flow_version,
            elapsed_ms=elapsed_ms,
        )


        page_flow_trace_response_data_item.additional_properties = d
        return page_flow_trace_response_data_item

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
