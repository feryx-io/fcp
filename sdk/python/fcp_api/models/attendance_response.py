from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.attendance_leg import AttendanceLeg
  from ..models.attendance_message import AttendanceMessage
  from ..models.attendance_question import AttendanceQuestion
  from ..models.attendance_registrant import AttendanceRegistrant
  from ..models.sitting_response import SittingResponse





T = TypeVar("T", bound="AttendanceResponse")



@_attrs_define
class AttendanceResponse:
    """ Who came to one sitting.

        Attributes:
            sitting (SittingResponse): One sitting of a meeting's room as the API shows it.
            attendees (int): Distinct people in the room (by registration, else by where they
                called from), the lobby aside.
            peak (int): The most legs in the room at once.
            legs (list[AttendanceLeg]):
            registrants (list[AttendanceRegistrant]): A webinar's registrations, each with whether they came; empty for a
                meeting.
            questions (list[AttendanceQuestion]):
            messages (list[AttendanceMessage]):
     """

    sitting: SittingResponse
    attendees: int
    peak: int
    legs: list[AttendanceLeg]
    registrants: list[AttendanceRegistrant]
    questions: list[AttendanceQuestion]
    messages: list[AttendanceMessage]
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.attendance_leg import AttendanceLeg # noqa: PLC0415
        from ..models.attendance_message import AttendanceMessage # noqa: PLC0415
        from ..models.attendance_question import AttendanceQuestion # noqa: PLC0415
        from ..models.attendance_registrant import AttendanceRegistrant # noqa: PLC0415
        from ..models.sitting_response import SittingResponse # noqa: PLC0415
        sitting = self.sitting.to_dict()

        attendees = self.attendees

        peak = self.peak

        legs = []
        for legs_item_data in self.legs:
            legs_item = legs_item_data.to_dict()
            legs.append(legs_item)



        registrants = []
        for registrants_item_data in self.registrants:
            registrants_item = registrants_item_data.to_dict()
            registrants.append(registrants_item)



        questions = []
        for questions_item_data in self.questions:
            questions_item = questions_item_data.to_dict()
            questions.append(questions_item)



        messages = []
        for messages_item_data in self.messages:
            messages_item = messages_item_data.to_dict()
            messages.append(messages_item)




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "sitting": sitting,
            "attendees": attendees,
            "peak": peak,
            "legs": legs,
            "registrants": registrants,
            "questions": questions,
            "messages": messages,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.attendance_leg import AttendanceLeg # noqa: PLC0415
        from ..models.attendance_message import AttendanceMessage # noqa: PLC0415
        from ..models.attendance_question import AttendanceQuestion # noqa: PLC0415
        from ..models.attendance_registrant import AttendanceRegistrant # noqa: PLC0415
        from ..models.sitting_response import SittingResponse # noqa: PLC0415
        d = dict(src_dict)
        sitting = SittingResponse.from_dict(d.pop("sitting"))




        attendees = d.pop("attendees")

        peak = d.pop("peak")

        legs = []
        _legs = d.pop("legs")
        for legs_item_data in (_legs):
            legs_item = AttendanceLeg.from_dict(legs_item_data)



            legs.append(legs_item)


        registrants = []
        _registrants = d.pop("registrants")
        for registrants_item_data in (_registrants):
            registrants_item = AttendanceRegistrant.from_dict(registrants_item_data)



            registrants.append(registrants_item)


        questions = []
        _questions = d.pop("questions")
        for questions_item_data in (_questions):
            questions_item = AttendanceQuestion.from_dict(questions_item_data)



            questions.append(questions_item)


        messages = []
        _messages = d.pop("messages")
        for messages_item_data in (_messages):
            messages_item = AttendanceMessage.from_dict(messages_item_data)



            messages.append(messages_item)


        attendance_response = cls(
            sitting=sitting,
            attendees=attendees,
            peak=peak,
            legs=legs,
            registrants=registrants,
            questions=questions,
            messages=messages,
        )


        attendance_response.additional_properties = d
        return attendance_response

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
