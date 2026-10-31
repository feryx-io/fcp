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
  from ..models.my_call_quality import MyCallQuality





T = TypeVar("T", bound="PageMyCallResponseDataItem")



@_attrs_define
class PageMyCallResponseDataItem:
    """ One of the user's calls, from their side.

        Attributes:
            cdr_id (str): The record's id, as `/cdrs` names it.
            call_id (str):
            direction (str): `outgoing` when the user placed it, else `incoming`.
            other_party (str): The other party's address.
            disposition (str): `answered`, `busy`, `no_answer`, `failed` or `cancelled`.
            missed (bool): An incoming call nobody answered.
            start_time (datetime.datetime):
            other_party_name (None | str | Unset):
            answer_time (datetime.datetime | None | Unset):
            end_time (datetime.datetime | None | Unset):
            duration_seconds (int | None | Unset): Talk time, from the answer.
            quality (MyCallQuality | None | Unset):
     """

    cdr_id: str
    call_id: str
    direction: str
    other_party: str
    disposition: str
    missed: bool
    start_time: datetime.datetime
    other_party_name: None | str | Unset = UNSET
    answer_time: datetime.datetime | None | Unset = UNSET
    end_time: datetime.datetime | None | Unset = UNSET
    duration_seconds: int | None | Unset = UNSET
    quality: MyCallQuality | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.my_call_quality import MyCallQuality # noqa: PLC0415
        cdr_id = self.cdr_id

        call_id = self.call_id

        direction = self.direction

        other_party = self.other_party

        disposition = self.disposition

        missed = self.missed

        start_time = self.start_time.isoformat()

        other_party_name: None | str | Unset
        if isinstance(self.other_party_name, Unset):
            other_party_name = UNSET
        else:
            other_party_name = self.other_party_name

        answer_time: None | str | Unset
        if isinstance(self.answer_time, Unset):
            answer_time = UNSET
        elif isinstance(self.answer_time, datetime.datetime):
            answer_time = self.answer_time.isoformat()
        else:
            answer_time = self.answer_time

        end_time: None | str | Unset
        if isinstance(self.end_time, Unset):
            end_time = UNSET
        elif isinstance(self.end_time, datetime.datetime):
            end_time = self.end_time.isoformat()
        else:
            end_time = self.end_time

        duration_seconds: int | None | Unset
        if isinstance(self.duration_seconds, Unset):
            duration_seconds = UNSET
        else:
            duration_seconds = self.duration_seconds

        quality: dict[str, Any] | None | Unset
        if isinstance(self.quality, Unset):
            quality = UNSET
        elif isinstance(self.quality, MyCallQuality):
            quality = self.quality.to_dict()
        else:
            quality = self.quality


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "cdr_id": cdr_id,
            "call_id": call_id,
            "direction": direction,
            "other_party": other_party,
            "disposition": disposition,
            "missed": missed,
            "start_time": start_time,
        })
        if other_party_name is not UNSET:
            field_dict["other_party_name"] = other_party_name
        if answer_time is not UNSET:
            field_dict["answer_time"] = answer_time
        if end_time is not UNSET:
            field_dict["end_time"] = end_time
        if duration_seconds is not UNSET:
            field_dict["duration_seconds"] = duration_seconds
        if quality is not UNSET:
            field_dict["quality"] = quality

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.my_call_quality import MyCallQuality # noqa: PLC0415
        d = dict(src_dict)
        cdr_id = d.pop("cdr_id")

        call_id = d.pop("call_id")

        direction = d.pop("direction")

        other_party = d.pop("other_party")

        disposition = d.pop("disposition")

        missed = d.pop("missed")

        start_time = datetime.datetime.fromisoformat(d.pop("start_time"))




        def _parse_other_party_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        other_party_name = _parse_other_party_name(d.pop("other_party_name", UNSET))


        def _parse_answer_time(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                answer_time_type_0 = datetime.datetime.fromisoformat(data)



                return answer_time_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        answer_time = _parse_answer_time(d.pop("answer_time", UNSET))


        def _parse_end_time(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                end_time_type_0 = datetime.datetime.fromisoformat(data)



                return end_time_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        end_time = _parse_end_time(d.pop("end_time", UNSET))


        def _parse_duration_seconds(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        duration_seconds = _parse_duration_seconds(d.pop("duration_seconds", UNSET))


        def _parse_quality(data: object) -> MyCallQuality | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                quality_type_0 = MyCallQuality.from_dict(data)



                return quality_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(MyCallQuality | None | Unset, data)

        quality = _parse_quality(d.pop("quality", UNSET))


        page_my_call_response_data_item = cls(
            cdr_id=cdr_id,
            call_id=call_id,
            direction=direction,
            other_party=other_party,
            disposition=disposition,
            missed=missed,
            start_time=start_time,
            other_party_name=other_party_name,
            answer_time=answer_time,
            end_time=end_time,
            duration_seconds=duration_seconds,
            quality=quality,
        )


        page_my_call_response_data_item.additional_properties = d
        return page_my_call_response_data_item

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
