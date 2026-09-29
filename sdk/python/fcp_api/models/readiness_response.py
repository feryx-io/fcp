from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast

if TYPE_CHECKING:
  from ..models.notification_readiness import NotificationReadiness
  from ..models.readiness_summary import ReadinessSummary
  from ..models.user_readiness import UserReadiness





T = TypeVar("T", bound="ReadinessResponse")



@_attrs_define
class ReadinessResponse:
    """ 
        Attributes:
            summary (ReadinessSummary):
            notifications (NotificationReadiness): Whether anyone would be told about an emergency call today.
            users (list[UserReadiness]):
     """

    summary: ReadinessSummary
    notifications: NotificationReadiness
    users: list[UserReadiness]
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.notification_readiness import NotificationReadiness # noqa: PLC0415
        from ..models.readiness_summary import ReadinessSummary # noqa: PLC0415
        from ..models.user_readiness import UserReadiness # noqa: PLC0415
        summary = self.summary.to_dict()

        notifications = self.notifications.to_dict()

        users = []
        for users_item_data in self.users:
            users_item = users_item_data.to_dict()
            users.append(users_item)




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "summary": summary,
            "notifications": notifications,
            "users": users,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.notification_readiness import NotificationReadiness # noqa: PLC0415
        from ..models.readiness_summary import ReadinessSummary # noqa: PLC0415
        from ..models.user_readiness import UserReadiness # noqa: PLC0415
        d = dict(src_dict)
        summary = ReadinessSummary.from_dict(d.pop("summary"))




        notifications = NotificationReadiness.from_dict(d.pop("notifications"))




        users = []
        _users = d.pop("users")
        for users_item_data in (_users):
            users_item = UserReadiness.from_dict(users_item_data)



            users.append(users_item)


        readiness_response = cls(
            summary=summary,
            notifications=notifications,
            users=users,
        )


        readiness_response.additional_properties = d
        return readiness_response

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
