from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="SipTraceResponse")



@_attrs_define
class SipTraceResponse:
    """ Response model for a single SIP trace message.

        Attributes:
            id (int):
            timestamp (datetime.datetime):
            direction (str):
            method_or_status (str):
            cseq (str):
            from_uri (str):
            to_uri (str):
            source_addr (str):
            dest_addr (str):
            transport (str):
            raw_message (str):
            node_id (None | str | Unset):
     """

    id: int
    timestamp: datetime.datetime
    direction: str
    method_or_status: str
    cseq: str
    from_uri: str
    to_uri: str
    source_addr: str
    dest_addr: str
    transport: str
    raw_message: str
    node_id: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        id = self.id

        timestamp = self.timestamp.isoformat()

        direction = self.direction

        method_or_status = self.method_or_status

        cseq = self.cseq

        from_uri = self.from_uri

        to_uri = self.to_uri

        source_addr = self.source_addr

        dest_addr = self.dest_addr

        transport = self.transport

        raw_message = self.raw_message

        node_id: None | str | Unset
        if isinstance(self.node_id, Unset):
            node_id = UNSET
        else:
            node_id = self.node_id


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "timestamp": timestamp,
            "direction": direction,
            "method_or_status": method_or_status,
            "cseq": cseq,
            "from_uri": from_uri,
            "to_uri": to_uri,
            "source_addr": source_addr,
            "dest_addr": dest_addr,
            "transport": transport,
            "raw_message": raw_message,
        })
        if node_id is not UNSET:
            field_dict["node_id"] = node_id

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        id = d.pop("id")

        timestamp = datetime.datetime.fromisoformat(d.pop("timestamp"))




        direction = d.pop("direction")

        method_or_status = d.pop("method_or_status")

        cseq = d.pop("cseq")

        from_uri = d.pop("from_uri")

        to_uri = d.pop("to_uri")

        source_addr = d.pop("source_addr")

        dest_addr = d.pop("dest_addr")

        transport = d.pop("transport")

        raw_message = d.pop("raw_message")

        def _parse_node_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        node_id = _parse_node_id(d.pop("node_id", UNSET))


        sip_trace_response = cls(
            id=id,
            timestamp=timestamp,
            direction=direction,
            method_or_status=method_or_status,
            cseq=cseq,
            from_uri=from_uri,
            to_uri=to_uri,
            source_addr=source_addr,
            dest_addr=dest_addr,
            transport=transport,
            raw_message=raw_message,
            node_id=node_id,
        )


        sip_trace_response.additional_properties = d
        return sip_trace_response

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
