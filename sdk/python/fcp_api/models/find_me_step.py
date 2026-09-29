from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="FindMeStep")



@_attrs_define
class FindMeStep:
    """ One step of find-me: these targets rung together for `ring_secs`.

        Attributes:
            targets (list[str]):
            ring_secs (int | Unset):
            confirm (bool | Unset): A leg of this step that answers must press 1 before it is
                connected: a mobile's voicemail does not take the call.
     """

    targets: list[str]
    ring_secs: int | Unset = UNSET
    confirm: bool | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        targets = self.targets



        ring_secs = self.ring_secs

        confirm = self.confirm


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "targets": targets,
        })
        if ring_secs is not UNSET:
            field_dict["ring_secs"] = ring_secs
        if confirm is not UNSET:
            field_dict["confirm"] = confirm

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        targets = cast(list[str], d.pop("targets"))


        ring_secs = d.pop("ring_secs", UNSET)

        confirm = d.pop("confirm", UNSET)

        find_me_step = cls(
            targets=targets,
            ring_secs=ring_secs,
            confirm=confirm,
        )


        find_me_step.additional_properties = d
        return find_me_step

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
