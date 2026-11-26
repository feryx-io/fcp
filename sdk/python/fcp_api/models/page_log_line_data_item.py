from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.page_log_line_data_item_fields import PageLogLineDataItemFields





T = TypeVar("T", bound="PageLogLineDataItem")



@_attrs_define
class PageLogLineDataItem:
    """ One line.

        Attributes:
            id (str): Its id in the service's stream; a page's cursor.
            service (str):
            time (datetime.datetime):
            level (str):
            target (str):
            message (str):
            fields (PageLogLineDataItemFields):
            kind (None | str | Unset): The service's node type, when it is up.
            tenant_id (None | str | Unset):
            call_id (None | str | Unset):
     """

    id: str
    service: str
    time: datetime.datetime
    level: str
    target: str
    message: str
    fields: PageLogLineDataItemFields
    kind: None | str | Unset = UNSET
    tenant_id: None | str | Unset = UNSET
    call_id: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.page_log_line_data_item_fields import PageLogLineDataItemFields # noqa: PLC0415
        id = self.id

        service = self.service

        time = self.time.isoformat()

        level = self.level

        target = self.target

        message = self.message

        fields = self.fields.to_dict()

        kind: None | str | Unset
        if isinstance(self.kind, Unset):
            kind = UNSET
        else:
            kind = self.kind

        tenant_id: None | str | Unset
        if isinstance(self.tenant_id, Unset):
            tenant_id = UNSET
        else:
            tenant_id = self.tenant_id

        call_id: None | str | Unset
        if isinstance(self.call_id, Unset):
            call_id = UNSET
        else:
            call_id = self.call_id


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "service": service,
            "time": time,
            "level": level,
            "target": target,
            "message": message,
            "fields": fields,
        })
        if kind is not UNSET:
            field_dict["kind"] = kind
        if tenant_id is not UNSET:
            field_dict["tenant_id"] = tenant_id
        if call_id is not UNSET:
            field_dict["call_id"] = call_id

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.page_log_line_data_item_fields import PageLogLineDataItemFields # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        service = d.pop("service")

        time = datetime.datetime.fromisoformat(d.pop("time"))




        level = d.pop("level")

        target = d.pop("target")

        message = d.pop("message")

        fields = PageLogLineDataItemFields.from_dict(d.pop("fields"))




        def _parse_kind(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        kind = _parse_kind(d.pop("kind", UNSET))


        def _parse_tenant_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        tenant_id = _parse_tenant_id(d.pop("tenant_id", UNSET))


        def _parse_call_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        call_id = _parse_call_id(d.pop("call_id", UNSET))


        page_log_line_data_item = cls(
            id=id,
            service=service,
            time=time,
            level=level,
            target=target,
            message=message,
            fields=fields,
            kind=kind,
            tenant_id=tenant_id,
            call_id=call_id,
        )


        page_log_line_data_item.additional_properties = d
        return page_log_line_data_item

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
