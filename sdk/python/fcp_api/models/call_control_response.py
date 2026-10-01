from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="CallControlResponse")



@_attrs_define
class CallControlResponse:
    """ What the node answered a control with.

        Attributes:
            call_id (str):
            status (str): `held`, `resumed`, `transferring`, `redirecting`, `playing` or
                `ended`.
            party (None | str | Unset): Who was held, transferred or redirected.
            target (None | str | Unset):
            flow_id (None | str | Unset):
            playback_id (None | str | Unset): A playback, for play and say.
     """

    call_id: str
    status: str
    party: None | str | Unset = UNSET
    target: None | str | Unset = UNSET
    flow_id: None | str | Unset = UNSET
    playback_id: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        call_id = self.call_id

        status = self.status

        party: None | str | Unset
        if isinstance(self.party, Unset):
            party = UNSET
        else:
            party = self.party

        target: None | str | Unset
        if isinstance(self.target, Unset):
            target = UNSET
        else:
            target = self.target

        flow_id: None | str | Unset
        if isinstance(self.flow_id, Unset):
            flow_id = UNSET
        else:
            flow_id = self.flow_id

        playback_id: None | str | Unset
        if isinstance(self.playback_id, Unset):
            playback_id = UNSET
        else:
            playback_id = self.playback_id


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "call_id": call_id,
            "status": status,
        })
        if party is not UNSET:
            field_dict["party"] = party
        if target is not UNSET:
            field_dict["target"] = target
        if flow_id is not UNSET:
            field_dict["flow_id"] = flow_id
        if playback_id is not UNSET:
            field_dict["playback_id"] = playback_id

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        call_id = d.pop("call_id")

        status = d.pop("status")

        def _parse_party(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        party = _parse_party(d.pop("party", UNSET))


        def _parse_target(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        target = _parse_target(d.pop("target", UNSET))


        def _parse_flow_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        flow_id = _parse_flow_id(d.pop("flow_id", UNSET))


        def _parse_playback_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        playback_id = _parse_playback_id(d.pop("playback_id", UNSET))


        call_control_response = cls(
            call_id=call_id,
            status=status,
            party=party,
            target=target,
            flow_id=flow_id,
            playback_id=playback_id,
        )


        call_control_response.additional_properties = d
        return call_control_response

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
