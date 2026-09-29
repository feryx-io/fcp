from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.executive_request import ExecutiveRequest
  from ..models.find_me import FindMe





T = TypeVar("T", bound="UpdateCallingRequest")



@_attrs_define
class UpdateCallingRequest:
    """ What may change. A forward given as an empty string is cleared; one
    left out is kept. A ring time of `0` clears it (the node's default).

        Attributes:
            dnd (bool | None | Unset):
            call_waiting (bool | None | Unset):
            forward_all (None | str | Unset):
            forward_busy (None | str | Unset):
            forward_no_answer (None | str | Unset):
            forward_unreachable (None | str | Unset):
            hold_music (None | str | Unset): A prompt of the tenant, `none`, or an empty string for the
                tenant's hold music.
            after_hours_schedule_id (None | str | Unset): A schedule of the tenant, or an empty string to clear it.
            after_hours_forward (None | str | Unset): A call target, or an empty string to clear it.
            find_me (FindMe | None | Unset):
            no_answer_secs (int | None | Unset):
            class_of_service_id (None | str | Unset): A class of service of the tenant, or an empty string for the
                tenant's default. An admin's to set: refused at `/me/calling`.
            executive (ExecutiveRequest | None | Unset):
     """

    dnd: bool | None | Unset = UNSET
    call_waiting: bool | None | Unset = UNSET
    forward_all: None | str | Unset = UNSET
    forward_busy: None | str | Unset = UNSET
    forward_no_answer: None | str | Unset = UNSET
    forward_unreachable: None | str | Unset = UNSET
    hold_music: None | str | Unset = UNSET
    after_hours_schedule_id: None | str | Unset = UNSET
    after_hours_forward: None | str | Unset = UNSET
    find_me: FindMe | None | Unset = UNSET
    no_answer_secs: int | None | Unset = UNSET
    class_of_service_id: None | str | Unset = UNSET
    executive: ExecutiveRequest | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.executive_request import ExecutiveRequest # noqa: PLC0415
        from ..models.find_me import FindMe # noqa: PLC0415
        dnd: bool | None | Unset
        if isinstance(self.dnd, Unset):
            dnd = UNSET
        else:
            dnd = self.dnd

        call_waiting: bool | None | Unset
        if isinstance(self.call_waiting, Unset):
            call_waiting = UNSET
        else:
            call_waiting = self.call_waiting

        forward_all: None | str | Unset
        if isinstance(self.forward_all, Unset):
            forward_all = UNSET
        else:
            forward_all = self.forward_all

        forward_busy: None | str | Unset
        if isinstance(self.forward_busy, Unset):
            forward_busy = UNSET
        else:
            forward_busy = self.forward_busy

        forward_no_answer: None | str | Unset
        if isinstance(self.forward_no_answer, Unset):
            forward_no_answer = UNSET
        else:
            forward_no_answer = self.forward_no_answer

        forward_unreachable: None | str | Unset
        if isinstance(self.forward_unreachable, Unset):
            forward_unreachable = UNSET
        else:
            forward_unreachable = self.forward_unreachable

        hold_music: None | str | Unset
        if isinstance(self.hold_music, Unset):
            hold_music = UNSET
        else:
            hold_music = self.hold_music

        after_hours_schedule_id: None | str | Unset
        if isinstance(self.after_hours_schedule_id, Unset):
            after_hours_schedule_id = UNSET
        else:
            after_hours_schedule_id = self.after_hours_schedule_id

        after_hours_forward: None | str | Unset
        if isinstance(self.after_hours_forward, Unset):
            after_hours_forward = UNSET
        else:
            after_hours_forward = self.after_hours_forward

        find_me: dict[str, Any] | None | Unset
        if isinstance(self.find_me, Unset):
            find_me = UNSET
        elif isinstance(self.find_me, FindMe):
            find_me = self.find_me.to_dict()
        else:
            find_me = self.find_me

        no_answer_secs: int | None | Unset
        if isinstance(self.no_answer_secs, Unset):
            no_answer_secs = UNSET
        else:
            no_answer_secs = self.no_answer_secs

        class_of_service_id: None | str | Unset
        if isinstance(self.class_of_service_id, Unset):
            class_of_service_id = UNSET
        else:
            class_of_service_id = self.class_of_service_id

        executive: dict[str, Any] | None | Unset
        if isinstance(self.executive, Unset):
            executive = UNSET
        elif isinstance(self.executive, ExecutiveRequest):
            executive = self.executive.to_dict()
        else:
            executive = self.executive


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
        })
        if dnd is not UNSET:
            field_dict["dnd"] = dnd
        if call_waiting is not UNSET:
            field_dict["call_waiting"] = call_waiting
        if forward_all is not UNSET:
            field_dict["forward_all"] = forward_all
        if forward_busy is not UNSET:
            field_dict["forward_busy"] = forward_busy
        if forward_no_answer is not UNSET:
            field_dict["forward_no_answer"] = forward_no_answer
        if forward_unreachable is not UNSET:
            field_dict["forward_unreachable"] = forward_unreachable
        if hold_music is not UNSET:
            field_dict["hold_music"] = hold_music
        if after_hours_schedule_id is not UNSET:
            field_dict["after_hours_schedule_id"] = after_hours_schedule_id
        if after_hours_forward is not UNSET:
            field_dict["after_hours_forward"] = after_hours_forward
        if find_me is not UNSET:
            field_dict["find_me"] = find_me
        if no_answer_secs is not UNSET:
            field_dict["no_answer_secs"] = no_answer_secs
        if class_of_service_id is not UNSET:
            field_dict["class_of_service_id"] = class_of_service_id
        if executive is not UNSET:
            field_dict["executive"] = executive

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.executive_request import ExecutiveRequest # noqa: PLC0415
        from ..models.find_me import FindMe # noqa: PLC0415
        d = dict(src_dict)
        def _parse_dnd(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        dnd = _parse_dnd(d.pop("dnd", UNSET))


        def _parse_call_waiting(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        call_waiting = _parse_call_waiting(d.pop("call_waiting", UNSET))


        def _parse_forward_all(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        forward_all = _parse_forward_all(d.pop("forward_all", UNSET))


        def _parse_forward_busy(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        forward_busy = _parse_forward_busy(d.pop("forward_busy", UNSET))


        def _parse_forward_no_answer(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        forward_no_answer = _parse_forward_no_answer(d.pop("forward_no_answer", UNSET))


        def _parse_forward_unreachable(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        forward_unreachable = _parse_forward_unreachable(d.pop("forward_unreachable", UNSET))


        def _parse_hold_music(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        hold_music = _parse_hold_music(d.pop("hold_music", UNSET))


        def _parse_after_hours_schedule_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        after_hours_schedule_id = _parse_after_hours_schedule_id(d.pop("after_hours_schedule_id", UNSET))


        def _parse_after_hours_forward(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        after_hours_forward = _parse_after_hours_forward(d.pop("after_hours_forward", UNSET))


        def _parse_find_me(data: object) -> FindMe | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                find_me_type_0 = FindMe.from_dict(data)



                return find_me_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(FindMe | None | Unset, data)

        find_me = _parse_find_me(d.pop("find_me", UNSET))


        def _parse_no_answer_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        no_answer_secs = _parse_no_answer_secs(d.pop("no_answer_secs", UNSET))


        def _parse_class_of_service_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        class_of_service_id = _parse_class_of_service_id(d.pop("class_of_service_id", UNSET))


        def _parse_executive(data: object) -> ExecutiveRequest | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                executive_type_0 = ExecutiveRequest.from_dict(data)



                return executive_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(ExecutiveRequest | None | Unset, data)

        executive = _parse_executive(d.pop("executive", UNSET))


        update_calling_request = cls(
            dnd=dnd,
            call_waiting=call_waiting,
            forward_all=forward_all,
            forward_busy=forward_busy,
            forward_no_answer=forward_no_answer,
            forward_unreachable=forward_unreachable,
            hold_music=hold_music,
            after_hours_schedule_id=after_hours_schedule_id,
            after_hours_forward=after_hours_forward,
            find_me=find_me,
            no_answer_secs=no_answer_secs,
            class_of_service_id=class_of_service_id,
            executive=executive,
        )


        update_calling_request.additional_properties = d
        return update_calling_request

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
