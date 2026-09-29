from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast






T = TypeVar("T", bound="PageDialogInfoDataItem")



@_attrs_define
class PageDialogInfoDataItem:
    """ Dialog information response.

        Attributes:
            call_id (str):
            local_tag (str):
            remote_tag (str):
            state (str):
            local_uri (str):
            remote_uri (str):
            remote_target (str):
            local_cseq (int):
            remote_cseq (int):
            route_set (list[str]):
     """

    call_id: str
    local_tag: str
    remote_tag: str
    state: str
    local_uri: str
    remote_uri: str
    remote_target: str
    local_cseq: int
    remote_cseq: int
    route_set: list[str]
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        call_id = self.call_id

        local_tag = self.local_tag

        remote_tag = self.remote_tag

        state = self.state

        local_uri = self.local_uri

        remote_uri = self.remote_uri

        remote_target = self.remote_target

        local_cseq = self.local_cseq

        remote_cseq = self.remote_cseq

        route_set = self.route_set




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "call_id": call_id,
            "local_tag": local_tag,
            "remote_tag": remote_tag,
            "state": state,
            "local_uri": local_uri,
            "remote_uri": remote_uri,
            "remote_target": remote_target,
            "local_cseq": local_cseq,
            "remote_cseq": remote_cseq,
            "route_set": route_set,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        call_id = d.pop("call_id")

        local_tag = d.pop("local_tag")

        remote_tag = d.pop("remote_tag")

        state = d.pop("state")

        local_uri = d.pop("local_uri")

        remote_uri = d.pop("remote_uri")

        remote_target = d.pop("remote_target")

        local_cseq = d.pop("local_cseq")

        remote_cseq = d.pop("remote_cseq")

        route_set = cast(list[str], d.pop("route_set"))


        page_dialog_info_data_item = cls(
            call_id=call_id,
            local_tag=local_tag,
            remote_tag=remote_tag,
            state=state,
            local_uri=local_uri,
            remote_uri=remote_uri,
            remote_target=remote_target,
            local_cseq=local_cseq,
            remote_cseq=remote_cseq,
            route_set=route_set,
        )


        page_dialog_info_data_item.additional_properties = d
        return page_dialog_info_data_item

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
